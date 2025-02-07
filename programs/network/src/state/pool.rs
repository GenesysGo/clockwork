use std::collections::VecDeque;

use anchor_lang::{prelude::*, AnchorDeserialize};
use clockwork_macros::TryFromData;

pub const SEED_POOL: &[u8] = b"pool";

const DEFAULT_POOL_SIZE: usize = 1;

/**
 * Pool
 */

#[account]
#[derive(Debug, TryFromData)]
pub struct Pool {
    pub id: u64,
    pub size: usize,
    pub workers: VecDeque<Pubkey>,
}

impl Pool {
    pub fn pubkey(id: u64) -> Pubkey {
        Pubkey::find_program_address(&[SEED_POOL, id.to_be_bytes().as_ref()], &crate::ID).0
    }
}

/**
 * PoolSettings
 */

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct PoolSettings {
    pub size: usize,
}

/**
 * PoolAccount
 */

pub trait PoolAccount {
    fn pubkey(&self) -> Pubkey;

    fn init(&mut self, id: u64) -> Result<()>;

    fn rotate(&mut self, worker: Pubkey) -> Result<()>;

    fn update(&mut self, settings: &PoolSettings) -> Result<()>;
}

impl PoolAccount for Account<'_, Pool> {
    fn pubkey(&self) -> Pubkey {
        Pool::pubkey(self.id)
    }

    fn init(&mut self, id: u64) -> Result<()> {
        self.id = id;
        self.size = DEFAULT_POOL_SIZE;
        self.workers = VecDeque::new();
        Ok(())
    }

    fn rotate(&mut self, worker: Pubkey) -> Result<()> {
        // Push new worker into the pool.
        self.workers.push_back(worker);

        // Drain pool to the configured size limit.
        while self.workers.len() > self.size {
            self.workers.pop_front();
        }

        Ok(())
    }

    fn update(&mut self, settings: &PoolSettings) -> Result<()> {
        self.size = settings.size;

        // Drain pool to the configured size limit.
        while self.workers.len() > self.size {
            self.workers.pop_front();
        }

        Ok(())
    }
}
