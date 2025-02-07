use anchor_lang::{prelude::*, AnchorDeserialize};
use clockwork_macros::TryFromData;

pub const SEED_SNAPSHOT_ENTRY: &[u8] = b"snapshot_entry";

/**
 * SnapshotEntry
 */

#[account]
#[derive(Debug, TryFromData)]
pub struct SnapshotEntry {
    pub delegation: Pubkey,
    pub id: u64,
    pub snapshot_frame: Pubkey,
    pub stake_amount: u64,
}

impl SnapshotEntry {
    pub fn pubkey(snapshot_frame: Pubkey, id: u64) -> Pubkey {
        Pubkey::find_program_address(
            &[
                SEED_SNAPSHOT_ENTRY,
                snapshot_frame.as_ref(),
                id.to_be_bytes().as_ref(),
            ],
            &crate::ID,
        )
        .0
    }
}

/**
 * SnapshotEntryAccount
 */

pub trait SnapshotEntryAccount {
    fn pubkey(&self) -> Pubkey;

    fn init(
        &mut self,
        delegation: Pubkey,
        id: u64,
        snapshot_frame: Pubkey,
        stake_amount: u64,
    ) -> Result<()>;
}

impl SnapshotEntryAccount for Account<'_, SnapshotEntry> {
    fn pubkey(&self) -> Pubkey {
        SnapshotEntry::pubkey(self.snapshot_frame, self.id)
    }

    fn init(
        &mut self,
        delegation: Pubkey,
        id: u64,
        snapshot_frame: Pubkey,
        stake_amount: u64,
    ) -> Result<()> {
        self.delegation = delegation;
        self.id = id;
        self.snapshot_frame = snapshot_frame;
        self.stake_amount = stake_amount;
        Ok(())
    }
}
