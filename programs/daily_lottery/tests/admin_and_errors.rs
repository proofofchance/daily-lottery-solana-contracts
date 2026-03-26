mod common;

use borsh::BorshDeserialize;
use common::TestContext;
use daily_lottery::*;
use solana_instruction::{AccountMeta, Instruction as SdkIx};
use solana_keypair::Keypair;
use solana_program::pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::program as system_program;
use std::io::Cursor;

fn read_after_disc<T: BorshDeserialize>(data: &[u8]) -> T {
    let mut cursor = Cursor::new(&data[8..]);
    T::deserialize_reader(&mut cursor).unwrap()
}

fn setup_lottery(
    ctx: &mut TestContext,
    program_id: Pubkey,
    authority: &Keypair,
) -> (Pubkey, Pubkey, Pubkey, Pubkey) {
    let (config_pda, _) = Pubkey::find_program_address(&[b"config"], &program_id);
    let init_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(authority.pubkey(), true),
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: borsh::to_vec(&Instruction::Initialize {
            ticket_price_lamports: 1_000_000,
            service_charge_bps: 500,
            max_winners_cap: 32,
        })
        .unwrap(),
    };
    ctx.send_tx(vec![init_ix], &[authority]).unwrap();

    let id_le = 1u64.to_le_bytes();
    let (lottery_pda, _) =
        Pubkey::find_program_address(&[b"lottery", config_pda.as_ref(), &id_le], &program_id);
    let (vault_pda, _) =
        Pubkey::find_program_address(&[b"vault", lottery_pda.as_ref()], &program_id);
    let (vote_tally_pda, _) =
        Pubkey::find_program_address(&[b"vote_tally", lottery_pda.as_ref()], &program_id);
    let create_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(authority.pubkey(), true),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: borsh::to_vec(&Instruction::CreateLottery).unwrap(),
    };
    ctx.send_tx(vec![create_ix], &[authority]).unwrap();

    (config_pda, lottery_pda, vault_pda, vote_tally_pda)
}

#[test]
fn update_service_charge_and_adjust_window() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority]);

    let (config_pda, lottery_pda, _, _) = setup_lottery(&mut ctx, program_id, &authority);

    let update_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(authority.pubkey(), true),
        ],
        data: borsh::to_vec(&Instruction::UpdateServiceCharge { new_bps: 250 }).unwrap(),
    };
    ctx.send_tx(vec![update_ix], &[&authority])
        .expect("update service charge should succeed");

    let adjust_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new_readonly(authority.pubkey(), true),
        ],
        data: borsh::to_vec(&Instruction::BeginRevealPhase).unwrap(),
    };
    ctx.send_tx(vec![adjust_ix], &[&authority]).unwrap();

    let cfg_acc = ctx.get_account(config_pda).unwrap();
    let cfg: Config = read_after_disc(&cfg_acc.data);
    assert_eq!(cfg.service_charge_bps, 250);
}

#[test]
fn buy_zero_tickets_should_fail() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let buyer = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer]);

    let (config_pda, lottery_pda, vault_pda, _) = setup_lottery(&mut ctx, program_id, &authority);
    let (participant_pda, _) = Pubkey::find_program_address(
        &[
            b"participant",
            lottery_pda.as_ref(),
            buyer.pubkey().as_ref(),
        ],
        &program_id,
    );
    let buy_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(participant_pda, false),
            AccountMeta::new(buyer.pubkey(), true),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: borsh::to_vec(&Instruction::BuyTickets {
            proof_of_chance_hash: Some([1u8; 32]),
            number_of_tickets: 0,
        })
        .unwrap(),
    };
    let result = ctx.send_tx(vec![buy_ix], &[&buyer]);
    assert!(result.is_err());
}

#[test]
fn unauthorized_service_charge_update_should_fail() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let attacker = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &attacker]);

    let (config_pda, _, _, _) = setup_lottery(&mut ctx, program_id, &authority);

    let update_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new_readonly(attacker.pubkey(), true),
        ],
        data: borsh::to_vec(&Instruction::UpdateServiceCharge { new_bps: 250 }).unwrap(),
    };
    let res = ctx.send_tx(vec![update_ix], &[&attacker]);
    assert!(res.is_err());
}

#[test]
fn invalid_reveal_window_should_fail() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority]);

    let (config_pda, lottery_pda, _, _) = setup_lottery(&mut ctx, program_id, &authority);

    let begin_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new_readonly(authority.pubkey(), true),
        ],
        data: borsh::to_vec(&Instruction::BeginRevealPhase).unwrap(),
    };
    ctx.send_tx(vec![begin_ix], &[&authority]).unwrap();
}

#[test]
fn settle_with_no_tickets_should_fail() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority]);

    let (config_pda, lottery_pda, vault_pda, vote_tally_pda) =
        setup_lottery(&mut ctx, program_id, &authority);

    let upload_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(authority.pubkey(), true),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(vote_tally_pda, false),
        ],
        data: borsh::to_vec(&Instruction::UploadReveals { entries: vec![] }).unwrap(),
    };
    let upload_res = ctx.send_tx(vec![upload_ix], &[&authority]);
    assert!(upload_res.is_err());

    let settle_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(authority.pubkey(), false),
            AccountMeta::new(authority.pubkey(), false),
        ],
        data: borsh::to_vec(&Instruction::FinalizeWinners).unwrap(),
    };
    let res = ctx.send_tx(vec![settle_ix], &[]);
    assert!(res.is_err());
}

#[test]
fn upload_reveals_mismatch_should_fail() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority]);

    let (config_pda, lottery_pda, _, vote_tally_pda) =
        setup_lottery(&mut ctx, program_id, &authority);

    let upload_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(authority.pubkey(), true),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(vote_tally_pda, false),
        ],
        data: borsh::to_vec(&Instruction::UploadReveals {
            entries: vec![(Pubkey::new_unique(), b"oops".to_vec())],
        })
        .unwrap(),
    };
    let res = ctx.send_tx(vec![upload_ix], &[&authority]);
    assert!(res.is_err());
}
