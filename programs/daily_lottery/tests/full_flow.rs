mod common;

use borsh::{to_vec, BorshDeserialize};
use common::TestContext;
use daily_lottery::*;
use solana_ed25519_program::new_ed25519_instruction_with_signature;
use solana_instruction::{AccountMeta, Instruction as SdkIx};
use solana_keypair::Keypair;
use solana_program::{pubkey::Pubkey, sysvar};
use solana_sha256_hasher::hash;
use solana_signer::Signer;
use solana_system_interface::program as system_program;
use std::io::Cursor;

fn read_after_disc<T: BorshDeserialize>(data: &[u8]) -> T {
    let mut cursor = Cursor::new(&data[8..]);
    T::deserialize_reader(&mut cursor).unwrap()
}

const ATTESTATION_MESSAGE_DOMAIN_V2: &[u8] = &[
    0x49, 0x4b, 0x49, 0x47, 0x41, 0x49, 0x5f, 0x41, 0x54, 0x54, 0x45, 0x53, 0x54, 0x5f, 0x56,
    0x32,
];

#[allow(dead_code)]
fn winning_index_from_entropy(entropy: [u8; 32], total_tickets: u64) -> u64 {
    if total_tickets == 0 {
        return 0;
    }
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&entropy[..16]);
    let num = u128::from_le_bytes(arr);
    (num % (total_tickets as u128)) as u64
}

#[test]
fn full_flow_attest_upload_settle() {
    let program_id = Pubkey::new_unique();

    let (config_pda, _) = Pubkey::find_program_address(&[b"config"], &program_id);
    let authority = Keypair::new();
    let buyer_a = Keypair::new();
    let buyer_b = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer_a, &buyer_b]);

    let init_ix = SdkIx {
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
    ctx.send_tx(vec![init_ix], &[&authority])
        .expect("initialize config");

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
        data: to_vec(&Instruction::CreateLottery).unwrap(),
    };
    ctx.send_tx(vec![create_ix], &[&authority])
        .expect("create lottery");

    let poc_a = b"apple";
    let hash_a = hash(poc_a).to_bytes();
    let poc_b = b"banana";
    let hash_b = hash(poc_b).to_bytes();
    let (part_a_pda, _) = Pubkey::find_program_address(
        &[
            b"participant",
            lottery_pda.as_ref(),
            buyer_a.pubkey().as_ref(),
        ],
        &program_id,
    );
    let (part_b_pda, _) = Pubkey::find_program_address(
        &[
            b"participant",
            lottery_pda.as_ref(),
            buyer_b.pubkey().as_ref(),
        ],
        &program_id,
    );
    let buy_a = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(part_a_pda, false),
            AccountMeta::new(buyer_a.pubkey(), true),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: to_vec(&Instruction::BuyTickets {
            proof_of_chance_hash: Some(hash_a),
            number_of_tickets: 2,
        })
        .unwrap(),
    };
    let buy_b = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(part_b_pda, false),
            AccountMeta::new(buyer_b.pubkey(), true),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: to_vec(&Instruction::BuyTickets {
            proof_of_chance_hash: Some(hash_b),
            number_of_tickets: 3,
        })
        .unwrap(),
    };
    for (ix, signer) in [(buy_a, &buyer_a), (buy_b, &buyer_b)] {
        ctx.send_tx(vec![ix], &[signer]).unwrap();
    }

    ctx.warp_to_slot(100_000);

    let begin_reveal_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new_readonly(authority.pubkey(), true),
        ],
        data: to_vec(&Instruction::BeginRevealPhase).unwrap(),
    };
    ctx.send_tx(vec![begin_reveal_ix], &[&authority]).unwrap();

    for (participant_pda, buyer, proof_hash) in [
        (part_a_pda, &buyer_a, hash_a),
        (part_b_pda, &buyer_b, hash_b),
    ] {
        let mut message = ATTESTATION_MESSAGE_DOMAIN_V2.to_vec();
        message.extend_from_slice(lottery_pda.as_ref());
        message.extend_from_slice(buyer.pubkey().as_ref());
        message.extend_from_slice(&proof_hash);
        message.extend_from_slice(&1u64.to_le_bytes());

        let signature = authority.sign_message(&message);
        let authority_pubkey = authority.pubkey().to_bytes();
        let ed25519_ix = new_ed25519_instruction_with_signature(
            &message,
            signature.as_array(),
            &authority_pubkey,
        );

        let attest_ix = SdkIx {
            program_id,
            accounts: vec![
                AccountMeta::new(config_pda, false),
                AccountMeta::new(lottery_pda, false),
                AccountMeta::new(participant_pda, false),
                AccountMeta::new_readonly(buyer.pubkey(), true),
                AccountMeta::new_readonly(sysvar::instructions::ID, false),
            ],
            data: to_vec(&Instruction::AttestUploaded {
                voted_number_of_winners: 1,
            })
            .unwrap(),
        };

        let attest_res = ctx.send_tx(vec![ed25519_ix, attest_ix], &[buyer]);
        assert!(attest_res.is_ok(), "attestation failed: {:?}", attest_res);
    }

    let upload_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(authority.pubkey(), true),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(vote_tally_pda, false),
            AccountMeta::new(part_a_pda, false),
            AccountMeta::new(part_b_pda, false),
        ],
        data: to_vec(&Instruction::UploadReveals {
            entries: vec![(part_a_pda, poc_a.to_vec()), (part_b_pda, poc_b.to_vec())],
        })
        .unwrap(),
    };
    ctx.send_tx(vec![upload_ix], &[&authority]).unwrap();

    ctx.warp_to_slot(200_000);

    let finalize_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new_readonly(authority.pubkey(), true),
            AccountMeta::new(part_a_pda, false),
            AccountMeta::new(part_b_pda, false),
        ],
        data: to_vec(&Instruction::FinalizeWinners).unwrap(),
    };
    ctx.send_tx(vec![finalize_ix], &[&authority]).unwrap();

    let lot_account = ctx.get_account(lottery_pda).unwrap();
    let lot: Lottery = read_after_disc(&lot_account.data);
    assert!(lot.winners_count > 0);
    assert!(lot.winners_merkle_root.iter().any(|&b| b != 0));
}
