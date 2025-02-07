use anchor_lang::{prelude::*, AnchorDeserialize};
use clockwork_macros::TryFromData;

pub const SEED_API: &[u8] = b"api";

/**
 * Api
 */

#[account]
#[derive(Debug, TryFromData)]
pub struct Api {
    pub ack_authority: Pubkey,
    pub authority: Pubkey,
    pub base_url: String,
    pub request_count: u64,
}

impl Api {
    pub fn pubkey(authority: Pubkey, base_url: String) -> Pubkey {
        Pubkey::find_program_address(
            &[SEED_API, authority.as_ref(), base_url.as_bytes()],
            &crate::ID,
        )
        .0
    }
}

/**
 * ApiAccount
 */

pub trait ApiAccount {
    fn pubkey(&self) -> Pubkey;

    fn init(&mut self, ack_authority: Pubkey, authority: Pubkey, base_url: String) -> Result<()>;
}

impl ApiAccount for Account<'_, Api> {
    fn pubkey(&self) -> Pubkey {
        Api::pubkey(self.authority, self.base_url.clone())
    }

    fn init(&mut self, ack_authority: Pubkey, authority: Pubkey, base_url: String) -> Result<()> {
        self.ack_authority = ack_authority;
        self.authority = authority;
        self.base_url = base_url;
        self.request_count = 0;
        Ok(())
    }
}
