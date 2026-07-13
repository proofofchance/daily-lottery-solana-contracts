#![allow(clippy::result_large_err, clippy::too_many_arguments)]

mod common;

use borsh::{BorshDeserialize, BorshSerialize};
use common::{assert_custom_error, TestContext};
use daily_lottery::{
    error::Error,
    instructions::settle_payout_batch::WinnerProof,
    state::{
        FinalizationLedger, WinnerPage, FINALIZATION_PHASE_AGGREGATING,
        FINALIZATION_PHASE_COMPLETED, FINALIZATION_PHASE_SELECTING, WINNERS_PER_PAGE,
    },
    *,
};
use solana_ed25519_program::new_ed25519_instruction_with_signature;
use solana_instruction::{AccountMeta, Instruction as SdkIx};
use solana_keypair::Keypair;
use solana_program::{clock::Clock, pubkey::Pubkey, sysvar};
use solana_sha256_hasher::hash;
use solana_signer::Signer;
use solana_system_interface::{instruction as system_instruction, program as system_program};
use std::io::Cursor;

fn read_after_disc<T: BorshDeserialize>(data: &[u8]) -> T {
    let mut cursor = Cursor::new(&data[8..]);
    T::deserialize_reader(&mut cursor).unwrap()
}

const ATTESTATION_MESSAGE_DOMAIN_V2: &[u8] = &[
    0x49, 0x4b, 0x49, 0x47, 0x41, 0x49, 0x5f, 0x41, 0x54, 0x54, 0x45, 0x53, 0x54, 0x5f, 0x56, 0x32,
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
    setup_lottery_with_max_winners_cap(ctx, program_id, authority, 32)
}

fn setup_lottery_with_max_winners_cap(
    ctx: &mut TestContext,
    program_id: Pubkey,
    authority: &Keypair,
    max_winners_cap: u32,
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
            max_winners_cap,
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
    attest_uploaded_result(
        ctx,
        program_id,
        config_pda,
        lottery_pda,
        participant_pda,
        participant,
        authority,
        proof_hash,
        voted_number_of_winners,
    )
    .unwrap();
}

fn attest_uploaded_result(
    ctx: &mut TestContext,
    program_id: Pubkey,
    config_pda: Pubkey,
    lottery_pda: Pubkey,
    participant_pda: Pubkey,
    participant: &Keypair,
    authority: &Keypair,
    proof_hash: [u8; 32],
    voted_number_of_winners: u64,
) -> litesvm::types::TransactionResult {
    let mut message = ATTESTATION_MESSAGE_DOMAIN_V2.to_vec();
    message.extend_from_slice(lottery_pda.as_ref());
    message.extend_from_slice(participant.pubkey().as_ref());
    message.extend_from_slice(&proof_hash);
    message.extend_from_slice(&voted_number_of_winners.to_le_bytes());

    let signature = authority.sign_message(&message);
    let authority_pubkey = authority.pubkey().to_bytes();
    let ed25519_ix =
        new_ed25519_instruction_with_signature(&message, signature.as_array(), &authority_pubkey);
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
    send_tx(ctx, vec![ed25519_ix, attest_ix], &[participant])
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

fn claim_refund(
    ctx: &mut TestContext,
    program_id: Pubkey,
    config_pda: Pubkey,
    lottery_pda: Pubkey,
    vault_pda: Pubkey,
    participant_pda: Pubkey,
    wallet: &Keypair,
) -> litesvm::types::TransactionResult {
    let ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(participant_pda, false),
            AccountMeta::new(wallet.pubkey(), true),
        ],
        data: borsh::to_vec(&Instruction::ClaimRefund).unwrap(),
    };
    send_tx(ctx, vec![ix], &[wallet])
}

fn close_participant(
    ctx: &mut TestContext,
    program_id: Pubkey,
    config_pda: Pubkey,
    lottery_pda: Pubkey,
    participant_pda: Pubkey,
    wallet: &Keypair,
) -> litesvm::types::TransactionResult {
    let ix = close_participant_ix(program_id, config_pda, lottery_pda, participant_pda, wallet);
    send_tx(ctx, vec![ix], &[wallet])
}

fn close_participant_ix(
    program_id: Pubkey,
    config_pda: Pubkey,
    lottery_pda: Pubkey,
    participant_pda: Pubkey,
    wallet: &Keypair,
) -> SdkIx {
    SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new_readonly(lottery_pda, false),
            AccountMeta::new(participant_pda, false),
            AccountMeta::new(wallet.pubkey(), true),
        ],
        data: borsh::to_vec(&Instruction::CloseParticipant).unwrap(),
    }
}

fn load_lottery(ctx: &mut TestContext, lottery_pda: Pubkey) -> Lottery {
    let lot_acc = ctx.get_account(lottery_pda).unwrap();
    read_after_disc(&lot_acc.data)
}

fn store_participant(ctx: &mut TestContext, participant_pda: Pubkey, participant: &Participant) {
    let mut account = ctx.get_account(participant_pda).unwrap();
    participant
        .serialize(&mut &mut account.data[8..])
        .expect("participant should serialize into existing account");
    ctx.set_account(participant_pda, account);
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

fn finalization_ledger_pda(program_id: &Pubkey, lottery_pda: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"finalization_root_v2", lottery_pda.as_ref()], program_id).0
}

fn winner_page_pda(program_id: &Pubkey, lottery_pda: &Pubkey, page_index: u32) -> Pubkey {
    Pubkey::find_program_address(
        &[
            b"winner_page",
            lottery_pda.as_ref(),
            &page_index.to_le_bytes(),
        ],
        program_id,
    )
    .0
}

fn load_winner(ctx: &mut TestContext, page_pda: Pubkey, offset: usize) -> Pubkey {
    let account = ctx.get_account(page_pda).expect("winner page account");
    let page: WinnerPage = read_after_disc(&account.data);
    page.winner(&account.data, offset)
        .expect("winner page entry")
        .wallet
}

fn load_finalization_ledger(
    ctx: &mut TestContext,
    finalization_ledger_pda: Pubkey,
) -> Option<FinalizationLedger> {
    let account = ctx.get_account(finalization_ledger_pda)?;
    if account.data.is_empty() {
        return None;
    }
    Some(read_after_disc(&account.data))
}

fn sorted_participants_by_wallet(ctx: &mut TestContext, participants: &[Pubkey]) -> Vec<Pubkey> {
    let mut keyed = participants
        .iter()
        .map(|participant_pda| {
            let account = ctx.get_account(*participant_pda).unwrap();
            let participant: Participant = read_after_disc(&account.data);
            (participant.wallet, *participant_pda)
        })
        .collect::<Vec<_>>();
    keyed.sort_by(|left, right| left.0.to_bytes().cmp(&right.0.to_bytes()));
    keyed
        .into_iter()
        .map(|(_, participant)| participant)
        .collect()
}

fn finalize_winners_chunk(
    ctx: &mut TestContext,
    program_id: Pubkey,
    config_pda: Pubkey,
    lottery_pda: Pubkey,
    vault_pda: Pubkey,
    authority: &Keypair,
    participant_accounts: &[Pubkey],
) -> litesvm::types::TransactionResult {
    let finalization_ledger = finalization_ledger_pda(&program_id, &lottery_pda);
    let page_index = load_finalization_ledger(ctx, finalization_ledger)
        .map(|root| root.selected_count / WINNERS_PER_PAGE as u32)
        .unwrap_or(0);
    let winner_page = winner_page_pda(&program_id, &lottery_pda, page_index);
    let mut accounts = vec![
        AccountMeta::new(config_pda, false),
        AccountMeta::new(lottery_pda, false),
        AccountMeta::new(vault_pda, false),
        AccountMeta::new(authority.pubkey(), true),
        AccountMeta::new_readonly(system_program::id(), false),
        AccountMeta::new(finalization_ledger, false),
        AccountMeta::new(winner_page, false),
    ];
    for participant in participant_accounts {
        accounts.push(AccountMeta::new(*participant, false));
    }

    let ix = SdkIx {
        program_id,
        accounts,
        data: borsh::to_vec(&Instruction::FinalizeWinners).unwrap(),
    };
    let next_slot = ctx.get_clock().slot.saturating_add(1);
    ctx.warp_to_slot(next_slot);
    let nonce_ix = system_instruction::transfer(
        &ctx.payer.pubkey(),
        &authority.pubkey(),
        (next_slot % 9).saturating_add(1),
    );
    send_tx(ctx, vec![nonce_ix, ix], &[authority])
}

fn finalize_winners_until_complete(
    ctx: &mut TestContext,
    program_id: Pubkey,
    config_pda: Pubkey,
    lottery_pda: Pubkey,
    vault_pda: Pubkey,
    authority: &Keypair,
    participants: &[Pubkey],
    chunk_size: usize,
) {
    let sorted_participants = sorted_participants_by_wallet(ctx, participants);
    let finalization_ledger = finalization_ledger_pda(&program_id, &lottery_pda);
    let mut iterations = 0usize;
    loop {
        let lot = load_lottery(ctx, lottery_pda);
        if lot.winners_count > 0 && lot.winners_merkle_root != [0u8; 32] {
            return;
        }
        iterations += 1;
        assert!(
            iterations <= sorted_participants.len().saturating_mul(260).max(4),
            "chunked finalization did not complete"
        );

        let ledger = load_finalization_ledger(ctx, finalization_ledger);
        let (start, required) = match ledger.as_ref().map(|ledger| ledger.phase) {
            None | Some(FINALIZATION_PHASE_AGGREGATING) => (
                ledger
                    .as_ref()
                    .map(|ledger| ledger.processed_count as usize)
                    .unwrap_or(0),
                sorted_participants.len(),
            ),
            Some(FINALIZATION_PHASE_SELECTING) => {
                let ledger = ledger.as_ref().unwrap();
                (
                    ledger.round_processed_count as usize,
                    ledger.eligible_count as usize,
                )
            }
            Some(FINALIZATION_PHASE_COMPLETED) => return,
            _ => panic!("unexpected finalization phase"),
        };
        assert!(
            start < required,
            "finalization cursor exceeded required count"
        );
        let end = (start + chunk_size.max(1)).min(required);
        finalize_winners_chunk(
            ctx,
            program_id,
            config_pda,
            lottery_pda,
            vault_pda,
            authority,
            &sorted_participants[start..end],
        )
        .unwrap();
    }
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
        vec![
            (participant_a, secret_a.to_vec()),
            (participant_b, secret_b.to_vec()),
        ],
        vec![participant_a, participant_b],
    );
    force_clock_after_upload_deadline(&mut ctx, lottery_pda);

    finalize_winners_until_complete(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &authority,
        &[participant_a, participant_b],
        2,
    );

    let lot = load_lottery(&mut ctx, lottery_pda);
    assert_eq!(lot.winners_count, 1);
    assert!(lot.winners_merkle_root.iter().any(|&b| b != 0));
    assert!(lot.total_payout > 0);
    assert!(lot.settlement_start_unix > 0);
}

#[test]
fn finalize_winners_keeps_non_revealing_buyers_eligible() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let buyer_a = Keypair::new();
    let buyer_b = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer_a, &buyer_b]);

    let (config_pda, lottery_pda, vault_pda, vote_tally_pda) =
        setup_lottery(&mut ctx, program_id, &authority);

    let secret_a = b"only-revealer";
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
        b"non-revealer",
        4,
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
        hash(secret_a).to_bytes(),
        1,
    );
    force_clock_after_upload_deadline(&mut ctx, lottery_pda);
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

    finalize_winners_until_complete(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &authority,
        &[participant_a, participant_b],
        1,
    );

    let lot = load_lottery(&mut ctx, lottery_pda);
    assert_eq!(lot.winners_count, 1);
    let ledger =
        load_finalization_ledger(&mut ctx, finalization_ledger_pda(&program_id, &lottery_pda))
            .unwrap();
    assert_eq!(ledger.eligible_count, 2);
    assert_eq!(ledger.total_eligible_tickets, 5);
}

#[test]
fn finalize_winners_selection_rejects_participant_missing_aggregation_inclusion() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let buyer_a = Keypair::new();
    let buyer_b = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer_a, &buyer_b]);

    let (config_pda, lottery_pda, vault_pda, vote_tally_pda) =
        setup_lottery(&mut ctx, program_id, &authority);

    let secret_a = b"selection-aggregation-a";
    let secret_b = b"selection-aggregation-b";
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
    force_clock_after_upload_deadline(&mut ctx, lottery_pda);

    let sorted_participants =
        sorted_participants_by_wallet(&mut ctx, &[participant_a, participant_b]);
    finalize_winners_chunk(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &authority,
        &sorted_participants,
    )
    .unwrap();

    let finalization_ledger = finalization_ledger_pda(&program_id, &lottery_pda);
    let ledger = load_finalization_ledger(&mut ctx, finalization_ledger).unwrap();
    assert_eq!(ledger.phase, FINALIZATION_PHASE_SELECTING);

    let corrupted_participant_pda = sorted_participants[0];
    let participant_account = ctx.get_account(corrupted_participant_pda).unwrap();
    let mut participant: Participant = read_after_disc(&participant_account.data);
    assert!(participant.reveal_included());
    assert_eq!(participant.finalization_generation, ledger.generation);
    participant.finalization_generation = 0;
    store_participant(&mut ctx, corrupted_participant_pda, &participant);

    let err = finalize_winners_chunk(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &authority,
        &[corrupted_participant_pda],
    )
    .expect_err("selection should reject participants not included during aggregation");
    assert_custom_error(err, Error::InvalidAccountData as u32);
}

#[test]
fn attest_uploaded_rejects_vote_above_configured_winner_cap() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let buyer_a = Keypair::new();
    let buyer_b = Keypair::new();
    let buyer_c = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer_a, &buyer_b, &buyer_c]);

    let (config_pda, lottery_pda, vault_pda, _) =
        setup_lottery_with_max_winners_cap(&mut ctx, program_id, &authority, 1);

    let secret_a = b"cap-a";
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
    buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer_b,
        b"cap-b",
        1,
    );
    buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer_c,
        b"cap-c",
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

    let err = attest_uploaded_result(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        participant_a,
        &buyer_a,
        &authority,
        hash(secret_a).to_bytes(),
        2,
    )
    .expect_err("vote above configured winner cap should fail");
    assert_custom_error(err, Error::InvalidInstruction as u32);
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

    let err = finalize_winners_chunk(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &authority,
        &[participant_a, participant_b],
    )
    .expect_err("FinalizeWinners should fail when attested_count is zero");
    assert_custom_error(err, Error::NoAttestedParticipants as u32);
}

#[test]
fn single_participant_attestation_accepts_vote_of_one() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let buyer = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer]);

    let (config_pda, lottery_pda, vault_pda, _) = setup_lottery(&mut ctx, program_id, &authority);
    let secret = b"single-attest";
    let proof_hash = hash(secret).to_bytes();
    let participant_pda = buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer,
        secret,
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
        participant_pda,
        &buyer,
        &authority,
        proof_hash,
        1,
    );

    let lot = load_lottery(&mut ctx, lottery_pda);
    assert_eq!(lot.attested_count, 1);
    let participant_account = ctx.get_account(participant_pda).unwrap();
    let participant: Participant = read_after_disc(&participant_account.data);
    assert!(participant.attested_uploaded);
    assert_eq!(participant.voted_number_of_winners, 1);
}

#[test]
fn single_participant_refund_can_settle_before_upload_deadline() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let buyer = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer]);

    let (config_pda, lottery_pda, vault_pda, _) = setup_lottery(&mut ctx, program_id, &authority);

    let participant_pda = buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer,
        b"single-refund",
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

    let lot = load_lottery(&mut ctx, lottery_pda);
    assert!(lot.uploads_complete);
    assert!(ctx.get_clock().unix_timestamp < lot.upload_deadline_unix);

    let finalize_no_attesters_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(authority.pubkey(), true),
        ],
        data: borsh::to_vec(&Instruction::FinalizeNoAttesters).unwrap(),
    };
    send_tx(&mut ctx, vec![finalize_no_attesters_ix], &[&authority]).unwrap();

    let lot = load_lottery(&mut ctx, lottery_pda);
    assert!(lot.settled);
    assert_eq!(lot.winners_count, 0);

    let buyer_before = ctx.get_account(buyer.pubkey()).unwrap().lamports;
    claim_refund(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        participant_pda,
        &buyer,
    )
    .unwrap();
    let buyer_after = ctx.get_account(buyer.pubkey()).unwrap().lamports;
    assert_eq!(buyer_after, buyer_before + 1_000_000);
}

#[test]
fn finalize_no_attesters_refund_path_still_works() {
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
        b"refund-a",
        1,
    );
    let participant_b = buy_tickets(
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
    let vault_before = ctx.get_account(vault_pda).unwrap().lamports;

    let finalize_no_attesters_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(authority.pubkey(), true),
        ],
        data: borsh::to_vec(&Instruction::FinalizeNoAttesters).unwrap(),
    };
    send_tx(&mut ctx, vec![finalize_no_attesters_ix], &[&authority]).unwrap();

    let lot = load_lottery(&mut ctx, lottery_pda);
    assert!(lot.settled);
    assert_eq!(lot.winners_count, 0);

    let authority_after = ctx.get_account(authority.pubkey()).unwrap().lamports;
    assert_eq!(
        authority_after, authority_before,
        "authority should not receive participant refund funds"
    );
    assert_eq!(ctx.get_account(vault_pda).unwrap().lamports, vault_before);

    let buyer_a_before = ctx.get_account(buyer_a.pubkey()).unwrap().lamports;
    claim_refund(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        participant_a,
        &buyer_a,
    )
    .unwrap();
    let buyer_a_after = ctx.get_account(buyer_a.pubkey()).unwrap().lamports;
    assert_eq!(buyer_a_after, buyer_a_before + 1_000_000);

    let double_claim_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(participant_a, false),
            AccountMeta::new(buyer_a.pubkey(), true),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        data: borsh::to_vec(&Instruction::ClaimRefund).unwrap(),
    };
    let double_claim = send_tx(&mut ctx, vec![double_claim_ix], &[&buyer_a])
        .expect_err("double refund claim should fail");
    assert_custom_error(double_claim, Error::RefundAlreadyClaimed as u32);

    let buyer_b_before = ctx.get_account(buyer_b.pubkey()).unwrap().lamports;
    claim_refund(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        participant_b,
        &buyer_b,
    )
    .unwrap();
    let buyer_b_after = ctx.get_account(buyer_b.pubkey()).unwrap().lamports;
    assert_eq!(buyer_b_after, buyer_b_before + 1_000_000);
}

#[test]
fn participant_can_close_after_refund_claim() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let buyer = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer]);

    let (config_pda, lottery_pda, vault_pda, _) = setup_lottery(&mut ctx, program_id, &authority);
    let participant_pda = buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer,
        b"close-participant-refund",
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

    let finalize_no_attesters_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(authority.pubkey(), true),
        ],
        data: borsh::to_vec(&Instruction::FinalizeNoAttesters).unwrap(),
    };
    send_tx(&mut ctx, vec![finalize_no_attesters_ix], &[&authority]).unwrap();

    let early_close = close_participant(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        participant_pda,
        &buyer,
    )
    .expect_err("participant rent should stay locked until refund claim");
    assert_custom_error(early_close, Error::InvalidLotteryState as u32);

    claim_refund(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        participant_pda,
        &buyer,
    )
    .unwrap();

    let participant_rent = ctx.get_account(participant_pda).unwrap().lamports;
    assert!(participant_rent > 0);
    let buyer_before = ctx.get_account(buyer.pubkey()).unwrap().lamports;
    let nonce_ix = system_instruction::transfer(&ctx.payer.pubkey(), &buyer.pubkey(), 1);
    let close_ix =
        close_participant_ix(program_id, config_pda, lottery_pda, participant_pda, &buyer);
    send_tx(&mut ctx, vec![nonce_ix, close_ix], &[&buyer]).unwrap();
    let buyer_after = ctx.get_account(buyer.pubkey()).unwrap().lamports;
    assert_eq!(buyer_after, buyer_before + participant_rent + 1);
    assert_eq!(
        ctx.get_account(participant_pda)
            .map(|account| account.lamports)
            .unwrap_or(0),
        0
    );
}

#[test]
fn refund_vault_can_close_after_all_refunds_claimed() {
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
        b"close-refund-a",
        1,
    );
    let participant_b = buy_tickets(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &buyer_b,
        b"close-refund-b",
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

    let finalize_no_attesters_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(authority.pubkey(), true),
        ],
        data: borsh::to_vec(&Instruction::FinalizeNoAttesters).unwrap(),
    };
    send_tx(&mut ctx, vec![finalize_no_attesters_ix], &[&authority]).unwrap();

    claim_refund(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        participant_a,
        &buyer_a,
    )
    .unwrap();

    let close_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(config_pda, false),
            AccountMeta::new_readonly(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(authority.pubkey(), false),
        ],
        data: borsh::to_vec(&Instruction::CloseRefundVault).unwrap(),
    };
    let early_close = send_tx(&mut ctx, vec![close_ix.clone()], &[])
        .expect_err("refund vault should stay open until every participant claims");
    assert_custom_error(early_close, Error::RefundUnavailable as u32);

    claim_refund(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        participant_b,
        &buyer_b,
    )
    .unwrap();

    let authority_before = ctx.get_account(authority.pubkey()).unwrap().lamports;
    let vault_rent = ctx.get_account(vault_pda).unwrap().lamports;
    assert!(vault_rent > 0);
    let nonce_ix = system_instruction::transfer(&ctx.payer.pubkey(), &authority.pubkey(), 1);
    send_tx(&mut ctx, vec![nonce_ix, close_ix], &[]).unwrap();
    let authority_after = ctx.get_account(authority.pubkey()).unwrap().lamports;
    assert_eq!(authority_after, authority_before + vault_rent + 1);
    assert_eq!(
        ctx.get_account(vault_pda)
            .map(|account| account.lamports)
            .unwrap_or(0),
        0
    );
}

#[test]
fn participants_above_legacy_cap_can_finalize_in_chunks() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let participant_count = 64usize;
    let buyers = (0..participant_count)
        .map(|_| Keypair::new())
        .collect::<Vec<_>>();
    let mut funded_accounts = Vec::with_capacity(participant_count + 1);
    funded_accounts.push(&authority);
    funded_accounts.extend(buyers.iter());
    let mut ctx = TestContext::new(program_id, &funded_accounts);

    let (config_pda, lottery_pda, vault_pda, vote_tally_pda) =
        setup_lottery(&mut ctx, program_id, &authority);

    let mut participants = Vec::with_capacity(participant_count);
    let mut secrets = Vec::with_capacity(participant_count);
    for (index, buyer) in buyers.iter().enumerate() {
        let secret = format!("scalable-participant-{index}");
        let participant = buy_tickets(
            &mut ctx,
            program_id,
            config_pda,
            lottery_pda,
            vault_pda,
            buyer,
            secret.as_bytes(),
            1,
        );
        participants.push(participant);
        secrets.push(secret.into_bytes());
    }

    begin_reveal_now(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        &authority,
        60,
        60,
    );

    for ((buyer, participant), secret) in buyers.iter().zip(participants.iter()).zip(secrets.iter())
    {
        attest_uploaded(
            &mut ctx,
            program_id,
            config_pda,
            lottery_pda,
            *participant,
            buyer,
            &authority,
            hash(secret).to_bytes(),
            1,
        );
    }

    for chunk in participants
        .iter()
        .copied()
        .zip(secrets.iter().cloned())
        .collect::<Vec<_>>()
        .chunks(4)
    {
        let entries = chunk
            .iter()
            .map(|(participant, secret)| (*participant, secret.clone()))
            .collect::<Vec<_>>();
        let accounts = chunk
            .iter()
            .map(|(participant, _)| *participant)
            .collect::<Vec<_>>();
        upload_reveals(
            &mut ctx,
            program_id,
            config_pda,
            lottery_pda,
            &authority,
            vote_tally_pda,
            entries,
            accounts,
        );
    }

    force_clock_after_upload_deadline(&mut ctx, lottery_pda);
    finalize_winners_until_complete(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &authority,
        &participants,
        2,
    );

    let lot = load_lottery(&mut ctx, lottery_pda);
    assert_eq!(lot.participants_count, participant_count as u64);
    assert_eq!(lot.provider_uploaded_count, participant_count as u64);
    assert_eq!(lot.winners_count, 1);
    assert_ne!(lot.winners_merkle_root, [0u8; 32]);
}

#[test]
fn participants_can_attest_with_onchain_reveal_without_provider_signature() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let buyer_a = Keypair::new();
    let buyer_b = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer_a, &buyer_b]);

    let (config_pda, lottery_pda, vault_pda, _) = setup_lottery(&mut ctx, program_id, &authority);

    let secret_a = b"self-a\x1fsalt";
    let secret_b = b"self-b\x1fsalt";
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

    for (buyer, participant, secret) in [
        (&buyer_a, participant_a, secret_a.as_slice()),
        (&buyer_b, participant_b, secret_b.as_slice()),
    ] {
        let ix = SdkIx {
            program_id,
            accounts: vec![
                AccountMeta::new_readonly(config_pda, false),
                AccountMeta::new(lottery_pda, false),
                AccountMeta::new(participant, false),
                AccountMeta::new_readonly(buyer.pubkey(), true),
            ],
            data: borsh::to_vec(&Instruction::AttestReveal {
                voted_number_of_winners: 1,
                reveal_plaintext: secret.to_vec(),
            })
            .unwrap(),
        };
        send_tx(&mut ctx, vec![ix], &[buyer]).unwrap();
    }

    let lot = load_lottery(&mut ctx, lottery_pda);
    assert_eq!(lot.attested_count, 2);
    assert_eq!(lot.provider_uploaded_count, 2);
    assert!(lot.uploads_complete);
    assert!(lot.settlement_start_unix > 0);

    finalize_winners_until_complete(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &authority,
        &[participant_a, participant_b],
        2,
    );

    let finalized = load_lottery(&mut ctx, lottery_pda);
    assert_eq!(finalized.winners_count, 1);
    assert_ne!(finalized.winners_merkle_root, [0u8; 32]);
}

#[test]
fn settle_payout_batch_does_not_require_authority_signature() {
    let program_id = Pubkey::new_unique();
    let authority = Keypair::new();
    let buyer_a = Keypair::new();
    let buyer_b = Keypair::new();
    let mut ctx = TestContext::new(program_id, &[&authority, &buyer_a, &buyer_b]);

    let (config_pda, lottery_pda, vault_pda, vote_tally_pda) =
        setup_lottery(&mut ctx, program_id, &authority);

    let secret_a = b"permissionless-settle-a";
    let secret_b = b"permissionless-settle-b";
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
        hash(secret_a).to_bytes(),
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
        hash(secret_b).to_bytes(),
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
    finalize_winners_until_complete(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &authority,
        &[participant_a, participant_b],
        2,
    );

    let lot = load_lottery(&mut ctx, lottery_pda);
    assert_eq!(lot.winners_count, 1);
    let winner_page_pda = winner_page_pda(&program_id, &lottery_pda, 0);
    let winner = load_winner(&mut ctx, winner_page_pda, 0);
    let winner_before = ctx.get_account(winner).unwrap().lamports;
    let authority_before = ctx.get_account(authority.pubkey()).unwrap().lamports;
    let vault_before = ctx.get_account(vault_pda).unwrap().lamports;
    let settle_ix = SdkIx {
        program_id,
        accounts: vec![
            AccountMeta::new(config_pda, false),
            AccountMeta::new(lottery_pda, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(authority.pubkey(), false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new(winner_page_pda, false),
            AccountMeta::new(winner, false),
        ],
        data: borsh::to_vec(&Instruction::SettlePayoutBatch {
            lottery_id: lot.id,
            batch_index: 0,
            winners: vec![WinnerProof {
                index: 0,
                recipient: winner,
                amount: lot.total_payout,
                merkle_proof: vec![],
            }],
        })
        .unwrap(),
    };
    send_tx(&mut ctx, vec![settle_ix], &[]).unwrap();

    let settled = load_lottery(&mut ctx, lottery_pda);
    assert!(settled.settlement_complete);
    assert_eq!(
        ctx.get_account(winner).unwrap().lamports,
        winner_before + lot.total_payout
    );
    assert!(
        ctx.get_account(authority.pubkey()).unwrap().lamports > authority_before,
        "authority should receive service fee, remainder, and vault rent"
    );
    assert_eq!(
        ctx.get_account(vault_pda)
            .map(|account| account.lamports)
            .unwrap_or(0),
        0
    );
    assert_eq!(
        winner_before
            + lot.total_payout
            + (ctx.get_account(authority.pubkey()).unwrap().lamports - authority_before),
        winner_before + vault_before
    );

    let participant_a_rent = ctx.get_account(participant_a).unwrap().lamports;
    let buyer_a_before_close = ctx.get_account(buyer_a.pubkey()).unwrap().lamports;
    close_participant(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        participant_a,
        &buyer_a,
    )
    .unwrap();
    assert_eq!(
        ctx.get_account(buyer_a.pubkey()).unwrap().lamports,
        buyer_a_before_close + participant_a_rent
    );
    assert_eq!(
        ctx.get_account(participant_a)
            .map(|account| account.lamports)
            .unwrap_or(0),
        0
    );
}

#[test]
fn settle_batch_with_mismatched_page_amount_fails() {
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

    finalize_winners_until_complete(
        &mut ctx,
        program_id,
        config_pda,
        lottery_pda,
        vault_pda,
        &authority,
        &[participant_a, participant_b],
        2,
    );

    let lot = load_lottery(&mut ctx, lottery_pda);
    assert!(lot.winners_count > 0);
    assert!(lot.total_payout > 0);

    let winner_page_pda = winner_page_pda(&program_id, &lottery_pda, 0);
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
            AccountMeta::new(winner_page_pda, false),
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
        .expect_err("settle payout should fail when the page amount does not match");
    assert_custom_error(err, Error::InvalidInstruction as u32);
}
