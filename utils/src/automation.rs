use std::{convert::TryFrom, fmt::Debug, hash::Hash};

use anchor_lang::{
    prelude::borsh::BorshSchema,
    prelude::Pubkey,
    prelude::*,
    solana_program::{self, instruction::Instruction},
    AnchorDeserialize,
};
use static_pubkey::static_pubkey;

/// The stand-in pubkey for delegating a payer address to a worker. All workers are re-imbursed by the user for lamports spent during this delegation.
pub static PAYER_PUBKEY: Pubkey = static_pubkey!("C1ockworkPayer11111111111111111111111111111");

/// The sighash of a named instruction in an Anchor program.
pub fn anchor_sighash(name: &str) -> [u8; 8] {
    let namespace = "global";
    let preimage = format!("{}:{}", namespace, name);
    let mut sighash = [0u8; 8];
    sighash.copy_from_slice(
        &anchor_lang::solana_program::hash::hash(preimage.as_bytes()).to_bytes()[..8],
    );
    sighash
}

/// The clock object, representing a specific moment in time recorded by a Solana cluster.
#[derive(AnchorDeserialize, AnchorSerialize, BorshSchema, Clone, Debug, PartialEq)]
pub struct ClockData {
    /// The current slot.
    pub slot: u64,
    /// The bank epoch.
    pub epoch: u64,
    /// The current unix timestamp.
    pub unix_timestamp: i64,
}

impl From<Clock> for ClockData {
    fn from(clock: Clock) -> Self {
        ClockData {
            slot: clock.slot,
            epoch: clock.epoch,
            unix_timestamp: clock.unix_timestamp,
        }
    }
}

impl TryFrom<Vec<u8>> for ClockData {
    type Error = Error;
    fn try_from(data: Vec<u8>) -> std::result::Result<Self, Self::Error> {
        Ok(
            borsh::try_from_slice_with_schema::<ClockData>(data.as_slice())
                .map_err(|_err| ErrorCode::AccountDidNotDeserialize)?,
        )
    }
}

/// The triggering conditions of a automation.
#[derive(AnchorDeserialize, AnchorSerialize, Debug, Clone, PartialEq)]
pub enum Trigger {
    /// Allows a automation to be kicked off whenever the data of an account changes.
    Account {
        /// The address of the account to monitor.
        address: Pubkey,
        /// The byte offset of the account data to monitor.
        offset: u64,
        /// The size of the byte slice to monitor (must be less than 1kb)
        size: u64,
    },

    /// Allows a automation to be kicked off according to a one-time or recurring schedule.
    Cron {
        /// The schedule in cron syntax. Value must be parsable by the `clockwork_cron` package.
        schedule: String,

        /// Boolean value indicating whether triggering moments may be skipped if they are missed (e.g. due to network downtime).
        /// If false, any "missed" triggering moments will simply be executed as soon as the network comes back online.
        skippable: bool,
    },

    /// Allows a automation to be kicked off as soon as it's created.
    Immediate,
}

/// A response value target programs can return to update the automation.
#[derive(AnchorDeserialize, AnchorSerialize, Clone, Debug)]
pub struct AutomationResponse {
    /// A dynamic instruction to execute next.
    pub next_instruction: Option<InstructionData>,
    /// Value to update the automation trigger to.
    pub trigger: Option<Trigger>,
}

impl Default for AutomationResponse {
    fn default() -> Self {
        return Self {
            next_instruction: None,
            trigger: None,
        };
    }
}

/// The data needed execute an instruction on Solana.
#[derive(AnchorDeserialize, AnchorSerialize, BorshSchema, Clone, Debug, Hash, PartialEq)]
pub struct InstructionData {
    /// Pubkey of the instruction processor that executes this instruction
    pub program_id: Pubkey,
    /// Metadata for what accounts should be passed to the instruction processor
    pub accounts: Vec<AccountMetaData>,
    /// Opaque data passed to the instruction processor
    pub data: Vec<u8>,
}

impl From<Instruction> for InstructionData {
    fn from(instruction: Instruction) -> Self {
        InstructionData {
            program_id: instruction.program_id,
            accounts: instruction
                .accounts
                .iter()
                .map(|a| AccountMetaData {
                    pubkey: a.pubkey,
                    is_signer: a.is_signer,
                    is_writable: a.is_writable,
                })
                .collect(),
            data: instruction.data,
        }
    }
}

impl From<&InstructionData> for Instruction {
    fn from(instruction: &InstructionData) -> Self {
        Instruction {
            program_id: instruction.program_id,
            accounts: instruction
                .accounts
                .iter()
                .map(|a| AccountMeta {
                    pubkey: a.pubkey,
                    is_signer: a.is_signer,
                    is_writable: a.is_writable,
                })
                .collect(),
            data: instruction.data.clone(),
        }
    }
}

impl TryFrom<Vec<u8>> for InstructionData {
    type Error = Error;
    fn try_from(data: Vec<u8>) -> std::result::Result<Self, Self::Error> {
        Ok(
            borsh::try_from_slice_with_schema::<InstructionData>(data.as_slice())
                .map_err(|_err| ErrorCode::AccountDidNotDeserialize)?,
        )
    }
}

/// Account metadata needed to execute an instruction on Solana.
#[derive(AnchorDeserialize, AnchorSerialize, BorshSchema, Clone, Debug, Hash, PartialEq)]
pub struct AccountMetaData {
    /// An account's public key
    pub pubkey: Pubkey,
    /// True if an Instruction requires a Transaction signature matching `pubkey`.
    pub is_signer: bool,
    /// True if the `pubkey` can be loaded as a read-write account.
    pub is_writable: bool,
}

impl AccountMetaData {
    /// Construct metadata for a writable account.
    pub fn new(pubkey: Pubkey, is_signer: bool) -> Self {
        Self {
            pubkey,
            is_signer,
            is_writable: true,
        }
    }

    /// Construct metadata for a read-only account.
    pub fn new_readonly(pubkey: Pubkey, is_signer: bool) -> Self {
        Self {
            pubkey,
            is_signer,
            is_writable: false,
        }
    }
}
