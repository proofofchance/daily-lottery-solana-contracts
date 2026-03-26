mod common;

use borsh::BorshDeserialize;
use common::{assert_custom_error, TestContext};
use daily_lottery::{error::Error, instructions::settle_payout_batch::WinnerProof, *};
use solana_ed25519_program::new_ed25519_instruction_with_signature;
use solana_instruction::{AccountMeta, Instruction as SdkIx};
use solana_keypair::Keypair;
use solana_program::{clock::Clock, pubkey::Pubkey, sysvar};
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

fn participant_pda(program_id: &Pubkey, lottery_pda: &Pubkey, wallet: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[b"participant", lottery_pda.as_ref(), wallet.as_ref()],
        program_id,
    )
    .0
}

fn send_tx(
    ctx: &mut TestContext,
    instructions: Vec<SdkIx>,
    signers: &[&Keypair],
) -> litesvm::types::TransactionResult {
    ctx.send_tx(instructions, signers)
}

fn setup_lottery(
    ctx: &mut TestContext,
    program_id: Pubkey,
    authority: &Keypair,
) -> (Pubkey, Pubkey, Pubkey, Pubkey) {
    let (config_pda, _) = Pubkey::find_program_address(&[b"config"], &program_id);
    let lottery_id = 1u64;
    let id_le = lottery_id.to_le_bytes();
    let (lottery_pda, _) =
        Pubkey::find_program_address(&[b"lottery", config_pda.as_ref(), &id_le], &program_id);
    let (vault_pda, _) =
        Pubkey::find_program_address(&[b"vault", lottery_pda.as_ref()], &program_id);
    let (vote_tally_pda, _) =
        Pubkey::find_program_address(&[b"vote_tally", lottery_pda.as_ref()], &program_id);

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
    send_tx(ctx, vec![init_ix], &[authority]).unwrap();

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
    send_tx(ctx, vec![create_ix], &[authority]).unwrap();

    (config_pda, lottery_pda, vault_pda, vote_tally_pda)
}

fn buy_tickets(
    ctx: &mut TestContext,
    program_id: Pubkey,
    config_pda: Pubkey,
    lottery_pda: Pubkey,
    vault_pda: Pubkey,
    buyer: &Keypair,
    secret: &[u8],
    tickets: u64,
) -> Pubkey {
    let participant = participant_pda(&program_id, &lottery_pda, &buyer.pubkey());
    let buy_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(participant, false),
            AccountMeta::new(buyer.pubkey(), true),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: borsh::to_vec(&Instruction::BuyTickets {
            proof_of_chance_hash: Some(hash(secret).to_bytes()),
            number_of_tickets: tickets,
        })
        .unwrap(),
    };
    send_tx(ctx, vec![buy_ix], &[buyer]).unwrap();
    participant
}

fn begin_reveal_now(
    ctx: &mut TestContext,
    program_id: Pubkey,
    config_pda: Pubkey,
    lottery_pda: Pubkey,
    authority: &Keypair,
    attestation_secs: u32,
    upload_secs: u32,
) {
    let ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new_readonly(authority.pubkey(), true),
        ],
        data: borsh::to_vec(&Instruction::BeginRevealNow {
            attestation_secs,
            upload_secs,
        })
        .unwrap(),
    };
    send_tx(ctx, vec![ix], &[authority]).unwrap();
}

fn attest_uploaded(
    ctx: &mut TestContext,
    program_id: Pubkey,
    config_pda: Pubkey,
    lottery_pda: Pubkey,
    participant_pda: Pubkey,
    participant: &Keypair,
    authority: &Keypair,
    proof_hash: [u8; 32],
    voted_number_of_winners: u64,
) {
    let mut message = ATTESTATION_MESSAGE_DOMAIN_V2.to_vec();
    message.extend_from_slice(lottery_pda.as_ref());
    message.extend_from_slice(participant.pubkey().as_ref());
    message.extend_from_slice(&proof_hash);
    message.extend_from_slice(&voted_number_of_winners.to_le_bytes());

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
            AccountMeta::new_readonly(participant.pubkey(), true),
            AccountMeta::new_readonly(sysvar::instructions::ID, false),
        ],
        data: borsh::to_vec(&Instruction::AttestUploaded {
            voted_number_of_winners,
        })
        .unwrap(),
    };
    send_tx(ctx, vec![ed25519_ix, attest_ix], &[participant]).unwrap();
}

fn upload_reveals(
    ctx: &mut TestContext,
    program_id: Pubkey,
    config_pda: Pubkey,
    lottery_pda: Pubkey,
    authority: &Keypair,
    vote_tally_pda: Pubkey,
    entries: Vec<(Pubkey, Vec<u8>)>,
    participant_accounts: Vec<Pubkey>,
) {
    let mut accounts = vec![
        AccountMeta::new(config_pda, false),
        AccountMeta::new(lottery_pda, false),
        AccountMeta::new(authority.pubkey(), true),
        AccountMeta::new_readonly(system_program::id(), false),
        AccountMeta::new(vote_tally_pda, false),
    ];
    for participant in participant_accounts {
        accounts.push(AccountMeta::new(participant, false));
    }

    let upload_ix = SdkIx {
        program_id,
        accounts,
        data: borsh::to_vec(&Instruction::UploadReveals { entries }).unwrap(),
    };
    send_tx(ctx, vec![upload_ix], &[authority]).unwrap();
}

fn load_lottery(ctx: &mut TestContext, lottery_pda: Pubkey) -> Lottery {
    let lot_acc = ctx.get_account(lottery_pda).unwrap();
    read_after_disc(&lot_acc.data)
}

fn force_clock_after_upload_deadline(ctx: &mut TestContext, lottery_pda: Pubkey) {
    let lot = load_lottery(ctx, lottery_pda);
    let mut clock: Clock = ctx.get_clock();
    if clock.unix_timestamp <= lot.upload_deadline_unix {
        clock.unix_timestamp = lot.upload_deadline_unix + 1;
        ctx.set_clock(&clock);
    }
    let updated_clock: Clock = ctx.get_clock();
    assert!(updated_clock.unix_timestamp > lot.upload_deadline_unix);
}

#[test]
fn finalize_winners_sets_fields() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let buyer_a = Keypair::new();
    let buyer_b = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer_a, &buyer_b]);

    let (config_pda, lottery_pda, vault_pda, vote_tally_pda) =
        setup_lottery(&mut ctx, program_id, &authority);

    let secret_a = b"alpha-secret";
    let secret_b = b"beta-secret";
    let proof_hash_a = hash(secret_a).to_bytes();
    let proof_hash_b = hash(secret_b).to_bytes();

    let participant_a = buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer_a,
        secret_a,
        3,
    );
    let participant_b = buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer_b,
        secret_b,
        2,
    );

    begin_reveal_now(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        &authority,
        60,
        60,
    );

    attest_uploaded(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        participant_a,
        &buyer_a,
        &authority,
        proof_hash_a,
        1,
    );
    attest_uploaded(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        participant_b,
        &buyer_b,
        &authority,
        proof_hash_b,
        1,
    );

    upload_reveals(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        &authority,
        vote_tally_pda,
        vec![(participant_a, secret_a.to_vec())],
        vec![participant_a],
    );
    force_clock_after_upload_deadline(&mut ctx, lottery_pda);

    let finalize_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new_readonly(authority.pubkey(), true),
            AccountMeta::new(participant_a, false),
            AccountMeta::new(participant_b, false),
        ],
        data: borsh::to_vec(&Instruction::FinalizeWinners).unwrap(),
    };
    send_tx(&mut ctx, vec![finalize_ix], &[&authority]).unwrap();

    let lot = load_lottery(&mut ctx, lottery_pda);
    assert_eq!(lot.winners_count, 1);
    assert!(lot.winners_merkle_root.iter().any(|&b| b != 0));
    assert!(lot.total_payout > 0);
    assert!(lot.settlement_start_unix > 0);
}

#[test]
fn finalize_winners_no_attesters_is_rejected() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let buyer_a = Keypair::new();
    let buyer_b = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer_a, &buyer_b]);

    let (config_pda, lottery_pda, vault_pda, _) = setup_lottery(&mut ctx, program_id, &authority);

    let participant_a = buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer_a,
        b"alpha-no-attest",
        3,
    );
    let participant_b = buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer_b,
        b"beta-no-attest",
        2,
    );

    let finalize_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new_readonly(authority.pubkey(), true),
            AccountMeta::new(participant_a, false),
            AccountMeta::new(participant_b, false),
        ],
        data: borsh::to_vec(&Instruction::FinalizeWinners).unwrap(),
    };
    let err = send_tx(&mut ctx, vec![finalize_ix], &[&authority])
        .expect_err("FinalizeWinners should fail when attested_count is zero");
    assert_custom_error(err, Error::NoAttestedParticipants as u32);
}

#[test]
fn finalize_no_attesters_refund_path_still_works() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let buyer_a = Keypair::new();
    let buyer_b = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer_a, &buyer_b]);

    let (config_pda, lottery_pda, vault_pda, _) = setup_lottery(&mut ctx, program_id, &authority);

    buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer_a,
        b"refund-a",
        1,
    );
    buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer_b,
        b"refund-b",
        1,
    );

    begin_reveal_now(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        &authority,
        1,
        1,
    );
    force_clock_after_upload_deadline(&mut ctx, lottery_pda);

    let authority_before = ctx.get_account(authority.pubkey()).unwrap().lamports;

    let finalize_no_attesters_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(authority.pubkey(), false),
        ],
        data: borsh::to_vec(&Instruction::FinalizeNoAttesters).unwrap(),
    };
    send_tx(&mut ctx, vec![finalize_no_attesters_ix], &[]).unwrap();

    let lot = load_lottery(&mut ctx, lottery_pda);
    assert!(lot.settled);
    assert_eq!(lot.winners_count, 0);

    if let Some(vault) = ctx.get_account(vault_pda) {
        assert_eq!(vault.lamports, 0);
    }

    let authority_after = ctx.get_account(authority.pubkey()).unwrap().lamports;
    assert!(
        authority_after > authority_before,
        "authority should receive refunded vault funds"
    );
}

#[test]
fn settle_batch_with_invalid_proof_fails() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let buyer_a = Keypair::new();
    let buyer_b = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer_a, &buyer_b]);

    let (config_pda, lottery_pda, vault_pda, vote_tally_pda) =
        setup_lottery(&mut ctx, program_id, &authority);

    let secret_a = b"proof-a";
    let secret_b = b"proof-b";
    let proof_hash_a = hash(secret_a).to_bytes();
    let proof_hash_b = hash(secret_b).to_bytes();

    let participant_a = buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer_a,
        secret_a,
        1,
    );
    let participant_b = buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer_b,
        secret_b,
        1,
    );

    begin_reveal_now(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        &authority,
        60,
        60,
    );

    attest_uploaded(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        participant_a,
        &buyer_a,
        &authority,
        proof_hash_a,
        1,
    );
    attest_uploaded(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        participant_b,
        &buyer_b,
        &authority,
        proof_hash_b,
        1,
    );

    upload_reveals(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        &authority,
        vote_tally_pda,
        vec![
            (participant_a, secret_a.to_vec()),
            (participant_b, secret_b.to_vec()),
        ],
        vec![participant_a, participant_b],
    );

    let finalize_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new_readonly(authority.pubkey(), true),
            AccountMeta::new(participant_a, false),
            AccountMeta::new(participant_b, false),
        ],
        data: borsh::to_vec(&Instruction::FinalizeWinners).unwrap(),
    };
    send_tx(&mut ctx, vec![finalize_ix], &[&authority]).unwrap();

    let lot = load_lottery(&mut ctx, lottery_pda);
    assert!(lot.winners_count > 0);
    assert!(lot.total_payout > 0);

    let (winners_ledger_pda, _) =
        Pubkey::find_program_address(&[b"winners_ledger", lottery_pda.as_ref()], &program_id);
    let invalid_winner = WinnerProof {
        index: 0,
        recipient: buyer_a.pubkey(),
        amount: lot.total_payout.saturating_add(1),
        merkle_proof: vec![],
    };
    let settle_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(authority.pubkey(), true),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(winners_ledger_pda, false),
            AccountMeta::new(buyer_a.pubkey(), false),
        ],
        data: borsh::to_vec(&Instruction::SettlePayoutBatch {
            lottery_id: lot.id,
            batch_index: 0,
            winners: vec![invalid_winner],
        })
        .unwrap(),
    };
    let err = send_tx(&mut ctx, vec![settle_ix], &[&authority])
        .expect_err("settle payout should fail with invalid merkle proof");
    assert_custom_error(err, Error::InvalidMerkleProof as u32);
}
