use {
    anchor_lang::{
        solana_program::{
            instruction::{AccountMeta, Instruction},
            pubkey::Pubkey,
        },
        InstructionData,
    },
    clockwork_network_program::state::*,
};

pub fn stake_delegations(automation: Pubkey) -> Instruction {
    Instruction {
        program_id: clockwork_network_program::ID,
        accounts: vec![
            AccountMeta::new_readonly(Config::pubkey(), false),
            AccountMeta::new(Registry::pubkey(), false),
            AccountMeta::new_readonly(automation, true),
        ],
        data: clockwork_network_program::instruction::StakeDelegationsJob {}.data(),
    }
}
