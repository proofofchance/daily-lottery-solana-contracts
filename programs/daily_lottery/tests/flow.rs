mod common;

use borsh::BorshDeserialize;
use common::TestContext;
use daily_lottery::*;
use solana_program::pubkey::Pubkey;
use solana_instruction::{AccountMeta, Instruction as SdkIx};
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_sha256_hasher::hash;
use solana_system_interface::program as system_program;
use std::io::Cursor;

fn read_after_disc<T: BorshDeserialize>(data: &[u8]) -> T {
    let mut cursor = Cursor::new(&data[8..]);
    T::deserialize_reader(&mut cursor).unwrap()
}

#[test]
fn create_and_buy_flow() {
    let program_id = Pubkey::new_unique();

    let authority = Keypair::new();
    let buyer = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer]);

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
    ctx.send_tx(vec![init_ix], &[&authority]).unwrap();

    let cfg_account = ctx.get_account(config_pda).expect("config account");
    assert_eq!(
        cfg_account.data.len(),
        daily_lottery::state::sizes::CONFIG_SIZE
    );
    assert_eq!(cfg_account.owner, program_id);
    let cfg: Config = read_after_disc(&cfg_account.data);
    assert_eq!(cfg.service_charge_bps, 500);

    let lottery_id: u64 = 1;
    let id_le = lottery_id.to_le_bytes();
    let (lottery_pda, _) =
        Pubkey::find_program_address(&[b"lottery", config_pda.as_ref(), &id_le], &program_id);
    let (vault_pda, _) =
        Pubkey::find_program_address(&[b"vault", lottery_pda.as_ref()], &program_id);

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
    ctx.send_tx(vec![create_ix], &[&authority]).unwrap();

    let poc_plain = b"hello world";
    let poc_hash = hash(poc_plain).to_bytes();
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
            proof_of_chance_hash: Some(poc_hash),
            number_of_tickets: 2,
        })
        .unwrap(),
    };
    let buy_res = ctx.send_tx(vec![buy_ix], &[&buyer]);
    assert!(buy_res.is_ok(), "buy_tickets failed: {:?}", buy_res);

    let lot_account = ctx.get_account(lottery_pda).expect("lottery account");
    let lot: Lottery = read_after_disc(&lot_account.data);
    assert_eq!(lot.total_tickets, 2);
    assert_eq!(lot.total_funds, 2_000_000);

    let part_account = ctx.get_account(participant_pda).expect("participant account");
    let part: Participant = read_after_disc(&part_account.data);
    assert_eq!(part.tickets_bought, 2);
    assert_eq!(part.proof_of_chance_hash, poc_hash);
}
