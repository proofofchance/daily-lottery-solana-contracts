//! Permissionless, fixed-account protocol-v2 daily-lottery finalization.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{
        Config, FinalizationLedger, Lottery, Participant, SelectedWinner, Vault, WinnerPage,
        FINALIZATION_PHASE_AGGREGATING, FINALIZATION_PHASE_COMPLETED, FINALIZATION_PHASE_SELECTING,
        FINALIZATION_PROTOCOL_VERSION, WINNERS_PER_PAGE,
    },
    utils::{
        account::{read_account_data, write_account_data},
        pda::{
            assert_pda_key, assert_pda_owned, derive_finalization_ledger_pda,
            derive_participant_pda, derive_winner_page_pda,
        },
        validation::{compute_service_fee, require_key_match, require_signer, require_writable},
    },
};
use sha2::{Digest, Sha256};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    program::invoke_signed,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::Sysvar,
};
use solana_system_interface::{instruction as system_instruction, program as system_program};
use std::fmt::Write as _;

pub const WINNER_ALGO_RULE_VERSION: &str = "reveal-plaintext-draw-v6-indexed";
const RPD_POOL_DOMAIN: &[u8] = b"IKIGAI_RPD_V3_POOL";
const RPD_SEED_DOMAIN: &[u8] = b"IKIGAI_RPD_V3_SEED";
const RPD_DRAW_DOMAIN: &[u8] = b"IKIGAI_RPD_V3_DRAW";

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let vault_ai = next_account_info(account_info_iter)?;
    let payer_ai = next_account_info(account_info_iter)?;
    let system_program_ai = next_account_info(account_info_iter)?;
    let finalization_root_ai = next_account_info(account_info_iter)?;
    let winner_page_ai = next_account_info(account_info_iter)?;
    let participant_accounts: Vec<&AccountInfo> = account_info_iter.collect();

    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(lottery_ai)?;
    require_writable(vault_ai)?;
    require_signer(payer_ai)?;
    require_writable(payer_ai)?;
    require_writable(finalization_root_ai)?;
    require_writable(winner_page_ai)?;
    require_key_match(system_program_ai, &system_program::id())?;

    let config: Config = read_account_data(config_ai)?;
    let mut lottery: Lottery = read_account_data(lottery_ai)?;
    let _vault: Vault = read_account_data(vault_ai)?;
    assert_pda_owned(
        program_id,
        lottery_ai,
        &[
            b"lottery",
            config_ai.key.as_ref(),
            &lottery.id.to_le_bytes(),
        ],
    )?;
    assert_pda_owned(program_id, vault_ai, &[b"vault", lottery_ai.key.as_ref()])?;
    require_key_match(vault_ai, &lottery.vault)?;

    if lottery.settlement_complete || lottery.settled {
        return Ok(());
    }
    if lottery.participants_count <= 1 {
        return Err(Error::InvalidInstruction.into());
    }
    if lottery.attested_count == 0 {
        return Err(Error::NoAttestedParticipants.into());
    }

    let current_time = Clock::get()?.unix_timestamp;
    let upload_elapsed =
        lottery.upload_deadline_unix > 0 && current_time > lottery.upload_deadline_unix;
    if lottery.has_missing_attested_reveals() {
        if upload_elapsed && lottery.remediation_start_unix == 0 {
            lottery.begin_remediation(current_time);
            write_account_data(lottery_ai, "Lottery", &lottery)?;
            LotteryEvent::RevealRemediationBegan {
                lottery_id: lottery.id,
                lottery: lottery_ai.key.to_string(),
                included_reveals_count: lottery.provider_uploaded_count,
                attested_count: lottery.attested_count,
                remediation_start_unix: lottery.remediation_start_unix,
                remediation_deadline_unix: lottery.remediation_deadline_unix,
                timestamp: current_time,
            }
            .emit();
            return Ok(());
        }
        return Err(Error::InvalidPhaseTransition.into());
    }
    if !lottery.uploads_complete && !upload_elapsed {
        return Err(Error::InvalidInstruction.into());
    }
    if participant_accounts.is_empty() {
        return Err(Error::MissingAccount.into());
    }

    let mut root = load_or_create_root(
        program_id,
        lottery_ai,
        payer_ai,
        system_program_ai,
        finalization_root_ai,
        &lottery,
        current_time,
    )?;

    let expected_page_index = root.selected_count / WINNERS_PER_PAGE as u32;
    assert_winner_page_key(program_id, lottery_ai, winner_page_ai, expected_page_index)?;

    let (batch_size, processed_count, required_count, completed) = match root.phase {
        FINALIZATION_PHASE_AGGREGATING => process_aggregation_chunk(
            program_id,
            lottery_ai,
            &config,
            &mut lottery,
            &mut root,
            &participant_accounts,
        )?,
        FINALIZATION_PHASE_SELECTING => process_selection_chunk(
            program_id,
            lottery_ai,
            payer_ai,
            system_program_ai,
            winner_page_ai,
            &mut lottery,
            &mut root,
            &participant_accounts,
            current_time,
        )?,
        FINALIZATION_PHASE_COMPLETED => return Ok(()),
        _ => return Err(Error::InvalidAccountData.into()),
    };

    LotteryEvent::FinalizationChunkProcessed {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        finalization_ledger: finalization_root_ai.key.to_string(),
        phase: phase_name(root.phase).to_string(),
        batch_size,
        processed_count,
        required_count,
        eligible_count: root.eligible_count,
        total_eligible_tickets: root.total_eligible_tickets,
        current_round: root.current_round as u64,
        target_winners: root.target_winners as u64,
        selected_winners_count: root.selected_count as u64,
        completed,
        timestamp: current_time,
    }
    .emit();

    write_account_data(finalization_root_ai, "FinalizationLedger", &root)?;
    write_account_data(lottery_ai, "Lottery", &lottery)?;
    Ok(())
}

fn load_or_create_root<'a>(
    program_id: &Pubkey,
    lottery_ai: &AccountInfo<'a>,
    payer_ai: &AccountInfo<'a>,
    system_program_ai: &AccountInfo<'a>,
    root_ai: &AccountInfo<'a>,
    lottery: &Lottery,
    current_time: i64,
) -> Result<FinalizationLedger, solana_program::program_error::ProgramError> {
    let (expected, bump) = derive_finalization_ledger_pda(program_id, lottery_ai.key);
    if expected != *root_ai.key {
        return Err(Error::InvalidSeeds.into());
    }
    if root_ai.data_is_empty() {
        let lamports = Rent::get()?.minimum_balance(FinalizationLedger::SIZE);
        let create_ix = system_instruction::create_account(
            payer_ai.key,
            root_ai.key,
            lamports,
            FinalizationLedger::SIZE as u64,
            program_id,
        );
        invoke_signed(
            &create_ix,
            &[payer_ai.clone(), root_ai.clone(), system_program_ai.clone()],
            &[&[b"finalization_root_v2", lottery_ai.key.as_ref(), &[bump]]],
        )?;
        return Ok(FinalizationLedger::new(
            *lottery_ai.key,
            lottery.participants_count,
            current_time,
        ));
    }
    assert_pda_owned(
        program_id,
        root_ai,
        &[b"finalization_root_v2", lottery_ai.key.as_ref()],
    )?;
    let root: FinalizationLedger = read_account_data(root_ai)?;
    if root.lottery != *lottery_ai.key
        || root.protocol_version != FINALIZATION_PROTOCOL_VERSION
        || root.required_count != lottery.participants_count
    {
        return Err(Error::InvalidAccountData.into());
    }
    Ok(root)
}

fn process_aggregation_chunk(
    program_id: &Pubkey,
    lottery_ai: &AccountInfo,
    config: &Config,
    lottery: &mut Lottery,
    root: &mut FinalizationLedger,
    participant_accounts: &[&AccountInfo],
) -> Result<(u64, u64, u64, bool), solana_program::program_error::ProgramError> {
    let mut batch_size = 0u64;
    for participant_ai in participant_accounts {
        require_writable(participant_ai)?;
        let mut participant = read_valid_participant(program_id, lottery_ai, participant_ai)?;
        require_participant_index(root.processed_count, participant.participant_index)?;
        if participant.tickets_bought == 0 || participant.finalization_generation == root.generation
        {
            return Err(Error::InvalidAccountData.into());
        }
        participant.mark_finalized_for_generation(root.generation);
        root.processed_count = root
            .processed_count
            .checked_add(1)
            .ok_or(Error::MathOverflow)?;
        if participant.reveal_included() {
            root.eligible_count = root
                .eligible_count
                .checked_add(1)
                .ok_or(Error::MathOverflow)?;
            root.total_eligible_tickets = root
                .total_eligible_tickets
                .checked_add(participant.tickets_bought)
                .ok_or(Error::MathOverflow)?;
            root.participants_commitment = extend_participants_commitment(
                root.participants_commitment,
                participant.participant_index,
                &participant.wallet,
                participant.tickets_bought,
                participant.reveal_digest,
            );
        }
        write_account_data(participant_ai, "Participant", &participant)?;
        batch_size = batch_size.checked_add(1).ok_or(Error::MathOverflow)?;
    }
    if root.processed_count > root.required_count {
        return Err(Error::InvalidInstruction.into());
    }
    if root.processed_count == root.required_count {
        if root.eligible_count == 0 || root.total_eligible_tickets == 0 {
            return Err(Error::NoAttestedParticipants.into());
        }
        let target = lottery
            .selected_number_of_winners
            .max(1)
            .min(config.effective_max_winners(lottery.participants_count))
            .min(root.eligible_count);
        root.target_winners = u32::try_from(target).map_err(|_| Error::MathOverflow)?;
        lottery.set_selected_winners(target)?;
        root.seed = compute_seed(
            lottery.id,
            root.eligible_count,
            root.total_eligible_tickets,
            lottery.poc_aggregate_hash,
            root.participants_commitment,
        );
        let draw = draw_index(&root.seed, 0, root.remaining_tickets()?)?;
        root.current_round = 0;
        root.begin_selection_round(draw);
    }
    Ok((batch_size, root.processed_count, root.required_count, false))
}

#[allow(clippy::too_many_arguments)]
fn process_selection_chunk<'a>(
    program_id: &Pubkey,
    lottery_ai: &AccountInfo<'a>,
    payer_ai: &AccountInfo<'a>,
    system_program_ai: &AccountInfo<'a>,
    winner_page_ai: &AccountInfo<'a>,
    lottery: &mut Lottery,
    root: &mut FinalizationLedger,
    participant_accounts: &[&AccountInfo<'a>],
    current_time: i64,
) -> Result<(u64, u64, u64, bool), solana_program::program_error::ProgramError> {
    let mut batch_size = 0u64;
    for participant_ai in participant_accounts {
        require_writable(participant_ai)?;
        let mut participant = read_valid_participant(program_id, lottery_ai, participant_ai)?;
        require_participant_index(root.round_processed_count, participant.participant_index)?;
        if participant.finalization_generation != root.generation || participant.tickets_bought == 0
        {
            return Err(Error::InvalidAccountData.into());
        }
        root.round_processed_count = root
            .round_processed_count
            .checked_add(1)
            .ok_or(Error::MathOverflow)?;
        if participant.reveal_included() && !participant.selected_in_generation(root.generation) {
            let start = root.round_remaining_tickets_seen;
            let end = start
                .checked_add(participant.tickets_bought)
                .ok_or(Error::MathOverflow)?;
            if !root.pending_winner_found
                && root.round_draw_index >= start
                && root.round_draw_index < end
            {
                root.pending_winner = participant.wallet;
                root.pending_winner_tickets = participant.tickets_bought;
                root.pending_winner_found = true;
                participant.mark_selected(root.generation, root.current_round);
                write_account_data(participant_ai, "Participant", &participant)?;
            }
            root.round_remaining_tickets_seen = end;
        }
        batch_size = batch_size.checked_add(1).ok_or(Error::MathOverflow)?;
    }
    if root.round_processed_count > root.required_count {
        return Err(Error::InvalidInstruction.into());
    }
    let mut completed = false;
    if root.round_processed_count == root.required_count {
        let expected = root.remaining_tickets()?;
        if root.round_remaining_tickets_seen != expected || !root.pending_winner_found {
            return Err(Error::WinnerNotFound.into());
        }
        let winner = SelectedWinner {
            wallet: root.pending_winner,
            tickets: root.pending_winner_tickets,
        };
        let winner_index = root.selected_count as u64;
        let page_index = root.selected_count / WINNERS_PER_PAGE as u32;
        let page_offset = root.selected_count % WINNERS_PER_PAGE as u32;
        let mut page = load_or_create_winner_page(
            program_id,
            lottery_ai,
            payer_ai,
            system_program_ai,
            winner_page_ai,
            root.generation,
            page_index,
        )?;
        {
            let mut page_data = winner_page_ai.try_borrow_mut_data()?;
            page.append(&mut page_data, winner)?;
        }
        write_account_data(winner_page_ai, "WinnerPage", &page)?;
        root.record_winner(winner)?;
        let (participant, _) = derive_participant_pda(program_id, lottery_ai.key, &winner.wallet);
        LotteryEvent::WinnerSelected {
            lottery_id: lottery.id,
            lottery: lottery_ai.key.to_string(),
            participant: participant.to_string(),
            winner: winner.wallet.to_string(),
            winner_index,
            tickets: winner.tickets,
            page_index,
            page_offset,
            timestamp: current_time,
        }
        .emit();

        if root.selected_count == root.target_winners {
            complete_finalization(lottery_ai, lottery, root, current_time)?;
            completed = true;
        } else {
            root.current_round = root
                .current_round
                .checked_add(1)
                .ok_or(Error::MathOverflow)?;
            let draw = draw_index(&root.seed, root.current_round, root.remaining_tickets()?)?;
            root.begin_selection_round(draw);
        }
    }
    let processed_count = if completed {
        root.required_count
    } else {
        root.round_processed_count
    };
    Ok((batch_size, processed_count, root.required_count, completed))
}

#[allow(clippy::too_many_arguments)]
fn load_or_create_winner_page<'a>(
    program_id: &Pubkey,
    lottery_ai: &AccountInfo<'a>,
    payer_ai: &AccountInfo<'a>,
    system_program_ai: &AccountInfo<'a>,
    page_ai: &AccountInfo<'a>,
    generation: u32,
    page_index: u32,
) -> Result<WinnerPage, solana_program::program_error::ProgramError> {
    let (_, bump) = derive_winner_page_pda(program_id, lottery_ai.key, page_index);
    assert_winner_page_key(program_id, lottery_ai, page_ai, page_index)?;
    if page_ai.data_is_empty() {
        let lamports = Rent::get()?.minimum_balance(WinnerPage::SIZE);
        let create_ix = system_instruction::create_account(
            payer_ai.key,
            page_ai.key,
            lamports,
            WinnerPage::SIZE as u64,
            program_id,
        );
        invoke_signed(
            &create_ix,
            &[payer_ai.clone(), page_ai.clone(), system_program_ai.clone()],
            &[&[
                b"winner_page",
                lottery_ai.key.as_ref(),
                &page_index.to_le_bytes(),
                &[bump],
            ]],
        )?;
        return Ok(WinnerPage::new(*lottery_ai.key, generation, page_index));
    }
    assert_pda_owned(
        program_id,
        page_ai,
        &[
            b"winner_page",
            lottery_ai.key.as_ref(),
            &page_index.to_le_bytes(),
        ],
    )?;
    let page: WinnerPage = read_account_data(page_ai)?;
    if page.lottery != *lottery_ai.key
        || page.generation != generation
        || page.page_index != page_index
    {
        return Err(Error::InvalidAccountData.into());
    }
    Ok(page)
}

fn assert_winner_page_key(
    program_id: &Pubkey,
    lottery_ai: &AccountInfo,
    page_ai: &AccountInfo,
    page_index: u32,
) -> ProgramResult {
    assert_pda_key(
        program_id,
        page_ai,
        &[
            b"winner_page",
            lottery_ai.key.as_ref(),
            &page_index.to_le_bytes(),
        ],
    )?;
    Ok(())
}

fn complete_finalization(
    lottery_ai: &AccountInfo,
    lottery: &mut Lottery,
    root: &mut FinalizationLedger,
    current_time: i64,
) -> ProgramResult {
    let winners_count = root.selected_count as u64;
    let service_fee = compute_service_fee(lottery.total_funds, lottery.service_charge_bps)?;
    let winners_pool = lottery.total_funds.saturating_sub(service_fee);
    if winners_count == 0 || winners_pool < winners_count {
        return Err(Error::InsufficientFunds.into());
    }
    let per_winner = winners_pool / winners_count;
    let total_payout = per_winner
        .checked_mul(winners_count)
        .ok_or(Error::MathOverflow)?;
    lottery.initialize_settlement(root.winners_commitment, winners_count, total_payout)?;
    if lottery.settlement_start_unix == 0 {
        lottery.settlement_start_unix = current_time;
    }
    root.complete(current_time);
    LotteryEvent::WinnersComputed {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        seed: bytes_to_hex(&root.seed),
        rule_version: WINNER_ALGO_RULE_VERSION.to_string(),
        total_eligible: root.eligible_count,
        winners: vec![],
        timestamp: current_time,
    }
    .emit();
    LotteryEvent::WinnersFinalized {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        winners_count,
        total_payout,
        per_winner_payout: per_winner,
        winners_merkle_root: root.winners_commitment,
        winners: vec![],
        timestamp: current_time,
    }
    .emit();
    Ok(())
}

fn read_valid_participant(
    program_id: &Pubkey,
    lottery_ai: &AccountInfo,
    participant_ai: &AccountInfo,
) -> Result<Participant, solana_program::program_error::ProgramError> {
    let participant: Participant = read_account_data(participant_ai)?;
    if participant.lottery != *lottery_ai.key {
        return Err(Error::InvalidAccountData.into());
    }
    assert_pda_owned(
        program_id,
        participant_ai,
        &[
            b"participant",
            lottery_ai.key.as_ref(),
            participant.wallet.as_ref(),
        ],
    )?;
    Ok(participant)
}

fn require_participant_index(expected: u64, actual: u64) -> Result<(), Error> {
    if actual != expected {
        return Err(Error::InvalidInstruction);
    }
    Ok(())
}

fn extend_participants_commitment(
    current: [u8; 32],
    participant_index: u64,
    wallet: &Pubkey,
    tickets: u64,
    reveal_digest: [u8; 32],
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(RPD_POOL_DOMAIN);
    h.update(current);
    h.update(participant_index.to_le_bytes());
    h.update(wallet.to_bytes());
    h.update(tickets.to_le_bytes());
    h.update(reveal_digest);
    h.finalize().into()
}

fn compute_seed(
    lottery_id: u64,
    eligible_count: u64,
    total_tickets: u64,
    aggregate: [u8; 32],
    participants_commitment: [u8; 32],
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(RPD_SEED_DOMAIN);
    h.update(lottery_id.to_le_bytes());
    h.update(eligible_count.to_le_bytes());
    h.update(total_tickets.to_le_bytes());
    h.update(aggregate);
    h.update(participants_commitment);
    h.finalize().into()
}

fn draw_index(seed: &[u8; 32], round: u32, remaining: u64) -> Result<u64, Error> {
    if remaining == 0 {
        return Err(Error::InvalidInstruction);
    }
    let modulus = remaining as u128;
    let rejection_floor = modulus.wrapping_neg() % modulus;
    for nonce in 0u32..=u32::MAX {
        let mut h = Sha256::new();
        h.update(RPD_DRAW_DOMAIN);
        h.update(seed);
        h.update((round as u64).to_le_bytes());
        h.update(nonce.to_le_bytes());
        let digest = h.finalize();
        let mut first = [0u8; 16];
        first.copy_from_slice(&digest[..16]);
        let sample = u128::from_le_bytes(first);
        if sample >= rejection_floor {
            return Ok((sample % modulus) as u64);
        }
    }
    Err(Error::WinnerSelectionFailed)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn phase_name(phase: u8) -> &'static str {
    match phase {
        FINALIZATION_PHASE_AGGREGATING => "aggregating",
        FINALIZATION_PHASE_SELECTING => "selecting",
        FINALIZATION_PHASE_COMPLETED => "completed",
        _ => "unknown",
    }
}
