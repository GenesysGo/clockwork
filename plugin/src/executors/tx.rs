use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use async_once::AsyncOnce;
use bincode::serialize;
use clockwork_client::{
    network::state::{Pool, Registry, Snapshot, SnapshotFrame, Worker},
    automation::state::Automation,
};
use lazy_static::lazy_static;
use log::info;
use solana_client::{
    nonblocking::{rpc_client::RpcClient, tpu_client::TpuClient},
    rpc_config::RpcSimulateTransactionConfig,
    tpu_client::TpuClientConfig,
};
use solana_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPluginError, Result as PluginResult,
};
use solana_program::{hash::Hash, message::Message, pubkey::Pubkey};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    signature::{Keypair, Signature},
    transaction::Transaction,
};
use tokio::{runtime::Runtime, sync::RwLock};

use crate::{config::PluginConfig, pool_position::PoolPosition, utils::read_or_new_keypair};

use super::AccountGet;

/// Number of slots to wait before checking for a confirmed transaction.
static TRANSACTION_CONFIRMATION_PERIOD: u64 = 10;

/// Number of slots to wait before trying to execute a automation while not in the pool.
static AUTOMATION_TIMEOUT_WINDOW: u64 = 8;

/// Number of times to retry a automation simulation.
static MAX_AUTOMATION_SIMULATION_FAILURES: u32 = 5;

/// The constant of the exponential backoff function.
static EXPONENTIAL_BACKOFF_CONSTANT: u32 = 2;

/// TxExecutor
pub struct TxExecutor {
    pub config: PluginConfig,
    pub executable_automations: RwLock<HashMap<Pubkey, ExecutableAutomationMetadata>>,
    pub transaction_history: RwLock<HashMap<Pubkey, TransactionMetadata>>,
    pub dropped_automations: AtomicU64,
    pub keypair: Keypair,
}

#[derive(Debug)]
pub struct ExecutableAutomationMetadata {
    pub due_slot: u64,
    pub simulation_failures: u32,
}

#[derive(Debug)]
pub struct TransactionMetadata {
    pub slot_sent: u64,
    pub signature: Signature,
}

impl TxExecutor {
    pub fn new(config: PluginConfig) -> Self {
        Self {
            config: config.clone(),
            executable_automations: RwLock::new(HashMap::new()),
            transaction_history: RwLock::new(HashMap::new()),
            dropped_automations: AtomicU64::new(0),
            keypair: read_or_new_keypair(config.keypath),
        }
    }

    pub async fn execute_txs(
        self: Arc<Self>,
        client: Arc<RpcClient>,
        automation_pubkeys: HashSet<Pubkey>,
        slot: u64,
        runtime: Arc<Runtime>,
    ) -> PluginResult<()> {
        // Index the provided automations as executable.
        let mut w_executable_automations = self.executable_automations.write().await;
        automation_pubkeys.iter().for_each(|pubkey| {
            w_executable_automations.insert(
                *pubkey,
                ExecutableAutomationMetadata {
                    due_slot: slot,
                    simulation_failures: 0,
                },
            );
        });

        // Drop automations that cross the simulation failure threshold.
        w_executable_automations.retain(|_automation_pubkey, metadata| {
            if metadata.simulation_failures > MAX_AUTOMATION_SIMULATION_FAILURES {
                self.dropped_automations.fetch_add(1, Ordering::Relaxed);
                false
            } else {
                true
            }
        });
        info!(
            "dropped_automations: {:?} executable_automations: {:?}",
            self.dropped_automations.load(Ordering::Relaxed),
            *w_executable_automations
        );
        drop(w_executable_automations);

        // Process retries.
        self.clone()
            .process_retries(client.clone(), slot)
            .await
            .ok();

        // Get self worker's position in the delegate pool.
        let worker_pubkey = Worker::pubkey(self.config.worker_id);
        if let Ok(pool_position) = client.get::<Pool>(&Pool::pubkey(0)).await.map(|pool| {
            let workers = &mut pool.workers.clone();
            PoolPosition {
                current_position: pool
                    .workers
                    .iter()
                    .position(|k| k.eq(&worker_pubkey))
                    .map(|i| i as u64),
                workers: workers.make_contiguous().to_vec().clone(),
            }
        }) {
            // Rotate into the worker pool.
            if pool_position.current_position.is_none() {
                self.clone()
                    .execute_pool_rotate_txs(client.clone(), slot, pool_position.clone())
                    .await
                    .ok();
            }

            // Execute automation transactions.
            self.clone()
                .execute_automation_exec_txs(client.clone(), slot, pool_position, runtime.clone())
                .await
                .ok();
        }

        Ok(())
    }

    async fn process_retries(
        self: Arc<Self>,
        client: Arc<RpcClient>,
        slot: u64,
    ) -> PluginResult<()> {
        // Get transaction signatures and corresponding automations to check.
        struct CheckableTransaction {
            automation_pubkey: Pubkey,
            signature: Signature,
        }
        let r_transaction_history = self.transaction_history.read().await;
        let checkable_transactions = r_transaction_history
            .iter()
            .filter(|(_, metadata)| slot > metadata.slot_sent + TRANSACTION_CONFIRMATION_PERIOD)
            .map(|(pubkey, metadata)| CheckableTransaction {
                automation_pubkey: *pubkey,
                signature: metadata.signature,
            })
            .collect::<Vec<CheckableTransaction>>();
        drop(r_transaction_history);

        // Lookup transaction statuses and track which automations are successful / retriable.
        let mut retriable_automations: HashSet<Pubkey> = HashSet::new();
        let mut successful_automations: HashSet<Pubkey> = HashSet::new();
        for data in checkable_transactions {
            match client
                .get_signature_status_with_commitment(
                    &data.signature,
                    CommitmentConfig::confirmed(),
                )
                .await
            {
                Err(_err) => {}
                Ok(status) => match status {
                    None => {
                        retriable_automations.insert(data.automation_pubkey);
                    }
                    Some(status) => match status {
                        Err(_err) => {
                            retriable_automations.insert(data.automation_pubkey);
                        }
                        Ok(()) => {
                            successful_automations.insert(data.automation_pubkey);
                        }
                    },
                },
            }
        }

        // Requeue retriable automations and drop transactions from history.
        let mut w_transaction_history = self.transaction_history.write().await;
        let mut w_executable_automations = self.executable_automations.write().await;
        for pubkey in successful_automations {
            w_transaction_history.remove(&pubkey);
        }
        for pubkey in retriable_automations {
            w_transaction_history.remove(&pubkey);
            w_executable_automations.insert(
                pubkey,
                ExecutableAutomationMetadata {
                    due_slot: slot,
                    simulation_failures: 0,
                },
            );
        }
        info!("transaction_history: {:?}", *w_transaction_history);
        drop(w_executable_automations);
        drop(w_transaction_history);
        Ok(())
    }

    async fn execute_pool_rotate_txs(
        self: Arc<Self>,
        client: Arc<RpcClient>,
        _slot: u64,
        pool_position: PoolPosition,
    ) -> PluginResult<()> {
        let registry = client.get::<Registry>(&Registry::pubkey()).await.unwrap();
        let snapshot_pubkey = Snapshot::pubkey(registry.current_epoch);
        let snapshot_frame_pubkey = SnapshotFrame::pubkey(snapshot_pubkey, self.config.worker_id);
        if let Ok(snapshot) = client.get::<Snapshot>(&snapshot_pubkey).await {
            if let Ok(snapshot_frame) = client.get::<SnapshotFrame>(&snapshot_frame_pubkey).await {
                if let Some(tx) = crate::builders::build_pool_rotation_tx(
                    client.clone(),
                    &self.keypair,
                    pool_position,
                    registry,
                    snapshot,
                    snapshot_frame,
                    self.config.worker_id,
                )
                .await
                {
                    self.clone().simulate_tx(&tx).await?;
                    self.clone().submit_tx(&tx).await?;
                }
            }
        }
        Ok(())
    }

    async fn get_executable_automations(
        self: Arc<Self>,
        pool_position: PoolPosition,
        slot: u64,
    ) -> PluginResult<Vec<Pubkey>> {
        // Get the set of automation pubkeys that are executable.
        // Note we parallelize using rayon because this work is CPU heavy.
        let r_executable_automations = self.executable_automations.read().await;
        let automation_pubkeys =
            if pool_position.current_position.is_none() && !pool_position.workers.is_empty() {
                // This worker is not in the pool. Get pubkeys of automations that are beyond the timeout window.
                r_executable_automations
                    .iter()
                    .filter(|(_pubkey, metadata)| slot > metadata.due_slot + AUTOMATION_TIMEOUT_WINDOW)
                    .filter(|(_pubkey, metadata)| {
                        slot >= metadata.due_slot
                            + EXPONENTIAL_BACKOFF_CONSTANT.pow(metadata.simulation_failures) as u64
                            - 1
                    })
                    .map(|(pubkey, _metadata)| *pubkey)
                    .collect::<Vec<Pubkey>>()
            } else {
                // This worker is in the pool. Get pubkeys executable automations.
                r_executable_automations
                    .iter()
                    .filter(|(_pubkey, metadata)| {
                        slot >= metadata.due_slot
                            + EXPONENTIAL_BACKOFF_CONSTANT.pow(metadata.simulation_failures) as u64
                            - 1
                    })
                    .map(|(pubkey, _metadata)| *pubkey)
                    .collect::<Vec<Pubkey>>()
            };
        drop(r_executable_automations);
        Ok(automation_pubkeys)
    }

    async fn execute_automation_exec_txs(
        self: Arc<Self>,
        client: Arc<RpcClient>,
        slot: u64,
        pool_position: PoolPosition,
        runtime: Arc<Runtime>,
    ) -> PluginResult<()> {
        let executable_automations = self
            .clone()
            .get_executable_automations(pool_position, slot)
            .await?;
        if executable_automations.is_empty() {
            return Ok(());
        }

        // Build transactions in parallel.
        // Note we parallelize using tokio because this work is IO heavy (RPC simulation calls).
        let tasks: Vec<_> = executable_automations
            .iter()
            .map(|automation_pubkey| {
                runtime.spawn(self.clone().try_build_automation_exec_tx(
                    client.clone(),
                    slot,
                    *automation_pubkey,
                ))
            })
            .collect();
        let mut executed_automations: HashMap<Pubkey, Signature> = HashMap::new();

        // Serialize to wire transactions.
        let wire_txs = futures::future::join_all(tasks)
            .await
            .iter()
            .filter_map(|res| match res {
                Err(_err) => None,
                Ok(res) => match res {
                    None => None,
                    Some((pubkey, tx)) => {
                        executed_automations.insert(*pubkey, tx.signatures[0]);
                        Some(tx)
                    }
                },
            })
            .map(|tx| serialize(tx).unwrap())
            .collect::<Vec<Vec<u8>>>();

        // Batch submit transactions to the leader.
        // TODO Explore rewriting the TPU client for optimized performance.
        //      This currently is by far the most expensive part of processing automations.
        //      Submitting transactions takes 8x longer (>200ms) than simulating and building transactions.
        match TPU_CLIENT
            .get()
            .await
            .try_send_wire_transaction_batch(wire_txs)
            .await
        {
            Err(err) => {
                info!("Failed to sent transaction batch: {:?}", err);
            }
            Ok(()) => {
                let mut w_executable_automations = self.executable_automations.write().await;
                let mut w_transaction_history = self.transaction_history.write().await;
                for (pubkey, signature) in executed_automations {
                    w_executable_automations.remove(&pubkey);
                    w_transaction_history.insert(
                        pubkey,
                        TransactionMetadata {
                            slot_sent: slot,
                            signature,
                        },
                    );
                }
                drop(w_executable_automations);
                drop(w_transaction_history);
            }
        }

        Ok(())
    }

    pub async fn try_build_automation_exec_tx(
        self: Arc<Self>,
        client: Arc<RpcClient>,
        slot: u64,
        automation_pubkey: Pubkey,
    ) -> Option<(Pubkey, Transaction)> {
        let automation = match client.clone().get::<Automation>(&automation_pubkey).await {
            Err(_err) => {
                self.increment_simulation_failure(automation_pubkey).await;
                return None;
            }
            Ok(automation) => automation,
        };

        if let Some(tx) = crate::builders::build_automation_exec_tx(
            client.clone(),
            &self.keypair,
            automation.clone(),
            automation_pubkey,
            self.config.worker_id,
        )
        .await
        {
            if self
                .clone()
                .dedupe_tx(slot, automation_pubkey, &tx)
                .await
                .is_ok()
            {
                Some((automation_pubkey, tx))
            } else {
                None
            }
        } else {
            self.increment_simulation_failure(automation_pubkey).await;
            None
        }
    }

    pub async fn increment_simulation_failure(self: Arc<Self>, automation_pubkey: Pubkey) {
        let mut w_executable_automations = self.executable_automations.write().await;
        w_executable_automations
            .entry(automation_pubkey)
            .and_modify(|metadata| metadata.simulation_failures += 1);
        drop(w_executable_automations);
    }

    pub async fn dedupe_tx(
        self: Arc<Self>,
        slot: u64,
        automation_pubkey: Pubkey,
        tx: &Transaction,
    ) -> PluginResult<()> {
        let r_transaction_history = self.transaction_history.read().await;
        if let Some(metadata) = r_transaction_history.get(&automation_pubkey) {
            if metadata.signature.eq(&tx.signatures[0]) && metadata.slot_sent.le(&slot) {
                return Err(GeyserPluginError::Custom(format!("Transaction signature is a duplicate of a previously submitted transaction").into()));
            }
        }
        drop(r_transaction_history);
        Ok(())
    }

    async fn simulate_tx(self: Arc<Self>, tx: &Transaction) -> PluginResult<Transaction> {
        TPU_CLIENT
            .get()
            .await
            .rpc_client()
            .simulate_transaction_with_config(
                tx,
                RpcSimulateTransactionConfig {
                    replace_recent_blockhash: false,
                    commitment: Some(CommitmentConfig::processed()),
                    ..RpcSimulateTransactionConfig::default()
                },
            )
            .await
            .map_err(|err| {
                GeyserPluginError::Custom(format!("Tx failed simulation: {}", err).into())
            })
            .map(|response| match response.value.err {
                None => Ok(tx.clone()),
                Some(err) => Err(GeyserPluginError::Custom(
                    format!(
                        "Tx failed simulation: {} Logs: {:#?}",
                        err, response.value.logs
                    )
                    .into(),
                )),
            })?
    }

    async fn submit_tx(self: Arc<Self>, tx: &Transaction) -> PluginResult<Transaction> {
        if !TPU_CLIENT.get().await.send_transaction(tx).await {
            return Err(GeyserPluginError::Custom(
                "Failed to send transaction".into(),
            ));
        }
        Ok(tx.clone())
    }
}

impl Debug for TxExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tx-executor")
    }
}

/// BlockhashAgnosticHash
trait BlockhashAgnosticHash {
    fn blockhash_agnostic_hash(&self) -> Hash;
}

impl BlockhashAgnosticHash for Message {
    fn blockhash_agnostic_hash(&self) -> Hash {
        Message {
            header: self.header.clone(),
            account_keys: self.account_keys.clone(),
            recent_blockhash: Hash::default(),
            instructions: self.instructions.clone(),
        }
        .hash()
    }
}

static LOCAL_RPC_URL: &str = "http://127.0.0.1:8899";
static LOCAL_WEBSOCKET_URL: &str = "ws://127.0.0.1:8900";

lazy_static! {
    static ref TPU_CLIENT: AsyncOnce<TpuClient> = AsyncOnce::new(async {
        let rpc_client = Arc::new(RpcClient::new_with_commitment(
            LOCAL_RPC_URL.into(),
            CommitmentConfig::processed(),
        ));
        let tpu_client = TpuClient::new(
            rpc_client,
            LOCAL_WEBSOCKET_URL.into(),
            TpuClientConfig::default(),
        )
        .await
        .unwrap();
        tpu_client
    });
}
