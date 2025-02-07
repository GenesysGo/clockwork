use anchor_lang::{prelude::*, AnchorDeserialize};
use clockwork_macros::TryFromData;

use super::Snapshot;

pub const SEED_EPOCH: &[u8] = b"epoch";

/**
 * Epoch
 */

#[account]
#[derive(Debug, TryFromData)]
pub struct Epoch {
    pub id: u64,
    pub snapshot: Pubkey,
}

impl Epoch {
    pub fn pubkey(id: u64) -> Pubkey {
        Pubkey::find_program_address(&[SEED_EPOCH, id.to_be_bytes().as_ref()], &crate::ID).0
    }
}

/**
 * EpochAccount
 */

pub trait EpochAccount {
    fn pubkey(&self) -> Pubkey;

    fn init(&mut self, id: u64) -> Result<()>;
}

impl EpochAccount for Account<'_, Epoch> {
    fn pubkey(&self) -> Pubkey {
        Epoch::pubkey(self.id)
    }

    fn init(&mut self, id: u64) -> Result<()> {
        self.id = id;
        self.snapshot = Snapshot::pubkey(self.pubkey());
        Ok(())
    }
}
