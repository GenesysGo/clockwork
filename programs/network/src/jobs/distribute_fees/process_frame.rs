use anchor_lang::prelude::*;
use clockwork_utils::automation::{
    anchor_sighash, AccountMetaData, InstructionData, AutomationResponse,
};

use crate::state::*;

#[derive(Accounts)]
pub struct DistributeFeesProcessFrame<'info> {
    #[account(address = Config::pubkey())]
    pub config: Account<'info, Config>,

    #[account(
        mut,
        seeds = [
            SEED_FEE,
            fee.worker.as_ref(),
        ],
        bump,
        has_one = worker,
    )]
    pub fee: Account<'info, Fee>,

    #[account(address = Registry::pubkey())]
    pub registry: Account<'info, Registry>,

    #[account(
        address = snapshot.pubkey(),
        constraint = snapshot.id.eq(&registry.current_epoch)
    )]
    pub snapshot: Account<'info, Snapshot>,

    #[account(
        address = snapshot_frame.pubkey(),
        has_one = snapshot,
        has_one = worker,
    )]
    pub snapshot_frame: Account<'info, SnapshotFrame>,

    #[account(address = config.epoch_automation)]
    pub automation: Signer<'info>,

    #[account(mut)]
    pub worker: Account<'info, Worker>,
}

pub fn handler(ctx: Context<DistributeFeesProcessFrame>) -> Result<AutomationResponse> {
    // Get accounts.
    let config = &ctx.accounts.config;
    let fee = &mut ctx.accounts.fee;
    let registry = &ctx.accounts.registry;
    let snapshot = &ctx.accounts.snapshot;
    let snapshot_frame = &ctx.accounts.snapshot_frame;
    let automation = &ctx.accounts.automation;
    let worker = &mut ctx.accounts.worker;

    // Calculate the fee account's usuable balance.
    let fee_lamport_balance = fee.to_account_info().lamports();
    let fee_data_len = 8 + fee.try_to_vec()?.len();
    let fee_rent_balance = Rent::get().unwrap().minimum_balance(fee_data_len);
    let fee_usable_balance = fee_lamport_balance.checked_sub(fee_rent_balance).unwrap();

    // Calculate the commission to be retained by the worker.
    let commission_balance = fee_usable_balance
        .checked_mul(worker.commission_rate)
        .unwrap()
        .checked_div(100)
        .unwrap();

    // Transfer commission to the worker.
    **fee.to_account_info().try_borrow_mut_lamports()? = fee
        .to_account_info()
        .lamports()
        .checked_sub(commission_balance)
        .unwrap();
    **worker.to_account_info().try_borrow_mut_lamports()? = worker
        .to_account_info()
        .lamports()
        .checked_add(commission_balance)
        .unwrap();

    // Increment the worker's commission balance.
    worker.commission_balance = worker
        .commission_balance
        .checked_add(commission_balance)
        .unwrap();

    // Record the balance that is distributable to delegations.
    fee.distributable_balance = fee_usable_balance.checked_sub(commission_balance).unwrap();

    // Build next instruction for the automation.
    let next_instruction = if snapshot_frame.total_entries.gt(&0) {
        // This snapshot frame has entries. Distribute fees to the delegations associated with the entries.
        let delegation_pubkey = Delegation::pubkey(worker.key(), 0);
        let snapshot_entry_pubkey = SnapshotEntry::pubkey(snapshot_frame.key(), 0);
        Some(InstructionData {
            program_id: crate::ID,
            accounts: vec![
                AccountMetaData::new_readonly(config.key(), false),
                AccountMetaData::new(delegation_pubkey, false),
                AccountMetaData::new(fee.key(), false),
                AccountMetaData::new_readonly(registry.key(), false),
                AccountMetaData::new_readonly(snapshot.key(), false),
                AccountMetaData::new_readonly(snapshot_entry_pubkey.key(), false),
                AccountMetaData::new_readonly(snapshot_frame.key(), false),
                AccountMetaData::new_readonly(automation.key(), true),
                AccountMetaData::new_readonly(worker.key(), false),
            ],
            data: anchor_sighash("distribute_fees_process_entry").to_vec(),
        })
    } else if snapshot_frame
        .id
        .checked_add(1)
        .unwrap()
        .lt(&snapshot.total_frames)
    {
        // This frame has no entries. Move on to the next frame.
        let next_worker_pubkey = Worker::pubkey(worker.id.checked_add(1).unwrap());
        let next_snapshot_frame_pubkey =
            SnapshotFrame::pubkey(snapshot.key(), snapshot_frame.id.checked_add(1).unwrap());
        Some(InstructionData {
            program_id: crate::ID,
            accounts: vec![
                AccountMetaData::new_readonly(config.key(), false),
                AccountMetaData::new(Fee::pubkey(next_worker_pubkey), false),
                AccountMetaData::new_readonly(registry.key(), false),
                AccountMetaData::new_readonly(snapshot.key(), false),
                AccountMetaData::new_readonly(next_snapshot_frame_pubkey, false),
                AccountMetaData::new_readonly(automation.key(), true),
                AccountMetaData::new(next_worker_pubkey, false),
            ],
            data: anchor_sighash("distribute_fees_process_frame").to_vec(),
        })
    } else {
        None
    };

    Ok(AutomationResponse {
        next_instruction,
        trigger: None,
    })
}
