mod common;

use borsh::to_vec;
use common::TestContext;
use daily_lottery::*;
use solana_account::Account;
use solana_instruction::{AccountMeta, Instruction as SdkIx};
use solana_keypair::Keypair;
use solana_program::pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::program as system_program;

#[test]
fn initialize_config() {
    let program_id = Pubkey::new_unique();
    let config_pda = Pubkey::find_program_address(&[b"config"], &program_id).0;
    let authority = Keypair::new();

    let mut ctx = TestContext::new(program_id, &[&authority]);
    ctx.set_account(
        config_pda,
        Account {
            lamports: 1_000_000_000,
            data: vec![0u8; 67],
            owner: program_id,
            executable: false,
            rent_epoch: 0,
        },
    );

    let ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(authority.pubkey(), true),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: to_vec(&Instruction::Initialize {
            ticket_price_lamports: 1_000_000,
            service_charge_bps: 500,
            max_winners_cap: 32,
        })
        .unwrap(),
    };

    ctx.send_tx(vec![ix], &[&authority]).unwrap();
}

#[test]
fn initialize_rejects_invalid_max_winners_cap() {
    for max_winners_cap in [0, daily_lottery::state::sizes::MAX_WINNERS as u32 + 1] {
        let program_id = Pubkey::new_unique();
        let config_pda = Pubkey::find_program_address(&[b"config"], &program_id).0;
        let authority = Keypair::new();
        let mut ctx = TestContext::new(program_id, &[&authority]);

        let ix = SdkIx {
            program_id,
            accounts: vec![
                AccountMeta::new(authority.pubkey(), true),
                AccountMeta::new(config_pda, false),
                AccountMeta::new_readonly(system_program::id(), false),
            ],
            data: to_vec(&Instruction::Initialize {
                ticket_price_lamports: 1_000_000,
                service_charge_bps: 500,
                max_winners_cap,
            })
            .unwrap(),
        };

        assert!(
            ctx.send_tx(vec![ix], &[&authority]).is_err(),
            "max_winners_cap={max_winners_cap} should be rejected"
        );
    }
}
