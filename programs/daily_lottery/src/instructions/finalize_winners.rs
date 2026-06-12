//! # Finalize Winners Instruction
//!
//! Computes lottery winners over chunked participant scans and stores the
//! winners merkle root on-chain. Finalization never requires every participant
//! account in one transaction.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{
        Config, FinalizationLedger, Lottery, Participant, Vault, FINALIZATION_PHASE_AGGREGATING,
        FINALIZATION_PHASE_COMPLETED, FINALIZATION_PHASE_SELECTING,
    },
    utils::{
        account::{read_account_data, write_account_data},
        pda::{assert_pda_key, assert_pda_owned, derive_finalization_ledger_pda},
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

pub const WINNER_ALGO_RULE_VERSION: &str = "reveal-plaintext-draw-v3-chunked";
const RPD_V3_POOL_DOMAIN: &[u8] = b"IKIGAI_RPD_V3_POOL";
const RPD_V3_SEED_DOMAIN: &[u8] = b"IKIGAI_RPD_V3_SEED";
const RPD_V3_DRAW_DOMAIN: &[u8] = b"IKIGAI_RPD_V3_DRAW";

/// Finalize winners and store commitment on-chain.
pub fn process(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let vault_ai = next_account_info(account_info_iter)?;
    let authority_ai = next_account_info(account_info_iter)?;
    let system_program_ai = next_account_info(account_info_iter)?;
    let finalization_ledger_ai = next_account_info(account_info_iter)?;

    let participant_accounts: Vec<&AccountInfo> = account_info_iter.collect();

    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(lottery_ai)?;
    require_writable(vault_ai)?;
    require_signer(authority_ai)?;
    require_writable(authority_ai)?;
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

    if authority_ai.key != &config.authority {
        return Err(Error::Unauthorized.into());
    }

    if lottery.settlement_complete || lottery.settled {
        return Err(Error::LotteryAlreadySettled.into());
    }
    if lottery.winners_count > 0
        || lottery.winners_merkle_root != [0u8; 32]
        || lottery.total_payout > 0
        || lottery.settlement_batches_completed > 0
    {
        return Err(Error::InvalidLotteryState.into());
    }

    if lottery.participants_count <= 1 {
        return Err(Error::InvalidInstruction.into());
    }

    if lottery.attested_count == 0 {
        return Err(Error::NoAttestedParticipants.into());
    }

    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;
    let current_phase = lottery.phase(current_time);
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

    if lottery.attested_reveals_complete() && !lottery.uploads_complete {
        lottery.uploads_complete = true;
    }

    if current_phase != "settlement" && current_phase != "settled" && !upload_elapsed {
        return Err(Error::InvalidInstruction.into());
    }

    if !lottery.uploads_complete && !upload_elapsed {
        return Err(Error::InvalidInstruction.into());
    }

    if lottery.total_tickets == 0 {
        lottery.settle();
        write_account_data(lottery_ai, "Lottery", &lottery)?;
        LotteryEvent::NoBuyersConcluded {
            lottery_id: lottery.id,
            lottery: lottery_ai.key.to_string(),
            timestamp: current_time,
        }
        .emit();
        return Ok(());
    }

    if participant_accounts.is_empty() {
        return Err(Error::MissingAccount.into());
    }

    let mut finalization_ledger = load_or_create_finalization_ledger(
        program_id,
        lottery_ai,
        authority_ai,
        system_program_ai,
        finalization_ledger_ai,
        &config,
        &lottery,
        current_time,
    )?;

    let (batch_size, processed_count, required_count, completed) = match finalization_ledger.phase {
        FINALIZATION_PHASE_AGGREGATING => process_aggregation_chunk(
            program_id,
            lottery_ai,
            &mut lottery,
            &mut finalization_ledger,
            &participant_accounts,
            current_time,
        )?,
        FINALIZATION_PHASE_SELECTING => process_selection_chunk(
            program_id,
            lottery_ai,
            &config,
            &mut lottery,
            &mut finalization_ledger,
            &participant_accounts,
            current_time,
        )?,
        FINALIZATION_PHASE_COMPLETED => return Err(Error::LotteryAlreadySettled.into()),
        _ => return Err(Error::InvalidAccountData.into()),
    };

    let phase = phase_name(finalization_ledger.phase).to_string();
    LotteryEvent::FinalizationChunkProcessed {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        finalization_ledger: finalization_ledger_ai.key.to_string(),
        phase,
        batch_size,
        processed_count,
        required_count,
        eligible_count: finalization_ledger.eligible_count,
        total_eligible_tickets: finalization_ledger.total_eligible_tickets,
        current_round: finalization_ledger.current_round,
        target_winners: finalization_ledger.target_winners,
        selected_winners_count: finalization_ledger.winners.len() as u64,
        completed,
        timestamp: current_time,
    }
    .emit();

    write_account_data(
        finalization_ledger_ai,
        "FinalizationLedger",
        &finalization_ledger,
    )?;
    write_account_data(lottery_ai, "Lottery", &lottery)?;

    Ok(())
}

fn load_or_create_finalization_ledger<'a>(
    program_id: &Pubkey,
    lottery_ai: &AccountInfo<'a>,
    authority_ai: &AccountInfo<'a>,
    system_program_ai: &AccountInfo<'a>,
    finalization_ledger_ai: &AccountInfo<'a>,
    config: &Config,
    lottery: &Lottery,
    current_time: i64,
) -> Result<FinalizationLedger, solana_program::program_error::ProgramError> {
    let (expected_ledger_pda, bump) = derive_finalization_ledger_pda(program_id, lottery_ai.key);
    if &expected_ledger_pda != finalization_ledger_ai.key {
        return Err(Error::InvalidSeeds.into());
    }

    let max_winners = config.effective_max_winners(lottery.participants_count);

    if finalization_ledger_ai.data_is_empty() {
        assert_pda_key(
            program_id,
            finalization_ledger_ai,
            &[b"finalization_ledger", lottery_ai.key.as_ref()],
        )?;
        let ledger_space = FinalizationLedger::size_for(max_winners as usize);
        let lamports = Rent::get()?.minimum_balance(ledger_space);
        let create_ix = system_instruction::create_account(
            authority_ai.key,
            finalization_ledger_ai.key,
            lamports,
            ledger_space as u64,
            program_id,
        );
        invoke_signed(
            &create_ix,
            &[
                authority_ai.clone(),
                finalization_ledger_ai.clone(),
                system_program_ai.clone(),
            ],
            &[&[b"finalization_ledger", lottery_ai.key.as_ref(), &[bump]]],
        )?;
        return Ok(FinalizationLedger::new(
            *lottery_ai.key,
            max_winners,
            current_time,
        ));
    }

    assert_pda_owned(
        program_id,
        finalization_ledger_ai,
        &[b"finalization_ledger", lottery_ai.key.as_ref()],
    )?;
    let ledger: FinalizationLedger = read_account_data(finalization_ledger_ai)?;
    if ledger.lottery != *lottery_ai.key || ledger.max_winners != max_winners {
        return Err(Error::InvalidAccountData.into());
    }

    Ok(ledger)
}

fn process_aggregation_chunk(
    program_id: &Pubkey,
    lottery_ai: &AccountInfo,
    lottery: &mut Lottery,
    finalization_ledger: &mut FinalizationLedger,
    participant_accounts: &[&AccountInfo],
    current_time: i64,
) -> Result<(u64, u64, u64, bool), solana_program::program_error::ProgramError> {
    let mut batch_size = 0u64;

    for participant_ai in participant_accounts.iter() {
        require_writable(participant_ai)?;
        let mut participant = read_valid_participant(program_id, lottery_ai, participant_ai)?;
        require_sorted_wallet(finalization_ledger, &participant.wallet)?;

        if !participant.reveal_included()
            || participant.settlement_included()
            || participant.tickets_bought == 0
        {
            return Err(Error::InvalidAccountData.into());
        }

        participant.mark_settlement_included();
        finalization_ledger.processed_count = finalization_ledger
            .processed_count
            .checked_add(1)
            .ok_or(Error::MathOverflow)?;
        finalization_ledger.eligible_count = finalization_ledger
            .eligible_count
            .checked_add(1)
            .ok_or(Error::MathOverflow)?;
        finalization_ledger.total_eligible_tickets = finalization_ledger
            .total_eligible_tickets
            .checked_add(participant.tickets_bought)
            .ok_or(Error::MathOverflow)?;
        finalization_ledger.participants_commitment = extend_participants_commitment(
            finalization_ledger.participants_commitment,
            &participant.wallet,
            participant.tickets_bought,
        );
        finalization_ledger.add_vote(
            participant.voted_winners(),
            participant.tickets_bought as u128,
            participant.attested_at_unix,
        );

        write_account_data(participant_ai, "Participant", &participant)?;
        batch_size = batch_size.checked_add(1).ok_or(Error::MathOverflow)?;
    }

    if finalization_ledger.processed_count > lottery.provider_uploaded_count {
        return Err(Error::InvalidInstruction.into());
    }

    let mut completed = false;
    if finalization_ledger.processed_count == lottery.provider_uploaded_count {
        if finalization_ledger.eligible_count == 0
            || finalization_ledger.total_eligible_tickets == 0
        {
            return Err(Error::WinnerNotFound.into());
        }

        let selected_count = finalization_ledger
            .selected_winner_count(lottery.participants_count)
            .min(finalization_ledger.eligible_count)
            .min(finalization_ledger.max_winners);
        if selected_count == 0 {
            return Err(Error::WinnerNotFound.into());
        }

        finalization_ledger.target_winners = selected_count;
        lottery.set_selected_winners(selected_count)?;
        finalization_ledger.seed = compute_seed_rpd_v3(
            lottery.id,
            finalization_ledger.eligible_count,
            finalization_ledger.total_eligible_tickets,
            lottery.poc_aggregate_hash,
            finalization_ledger.participants_commitment,
        );
        let draw_index = draw_index_rpd_v3(
            &finalization_ledger.seed,
            0,
            finalization_ledger.remaining_tickets()?,
        )?;
        finalization_ledger.current_round = 0;
        finalization_ledger.begin_selection_round(draw_index);

        if lottery.settlement_start_unix == 0 {
            lottery.settlement_start_unix = current_time;
        }
        completed = false;
    }

    Ok((
        batch_size,
        finalization_ledger.processed_count,
        lottery.provider_uploaded_count,
        completed,
    ))
}

fn process_selection_chunk(
    program_id: &Pubkey,
    lottery_ai: &AccountInfo,
    config: &Config,
    lottery: &mut Lottery,
    finalization_ledger: &mut FinalizationLedger,
    participant_accounts: &[&AccountInfo],
    current_time: i64,
) -> Result<(u64, u64, u64, bool), solana_program::program_error::ProgramError> {
    let mut batch_size = 0u64;

    for participant_ai in participant_accounts.iter() {
        let participant = read_valid_participant(program_id, lottery_ai, participant_ai)?;
        require_sorted_wallet(finalization_ledger, &participant.wallet)?;

        if !participant.reveal_included()
            || !participant.settlement_included()
            || participant.tickets_bought == 0
        {
            return Err(Error::InvalidAccountData.into());
        }

        finalization_ledger.round_processed_count = finalization_ledger
            .round_processed_count
            .checked_add(1)
            .ok_or(Error::MathOverflow)?;

        if !finalization_ledger.has_selected(&participant.wallet) {
            let start = finalization_ledger.round_remaining_tickets_seen;
            let end = start
                .checked_add(participant.tickets_bought)
                .ok_or(Error::MathOverflow)?;
            if !finalization_ledger.round_winner_found
                && finalization_ledger.round_draw_index >= start
                && finalization_ledger.round_draw_index < end
            {
                finalization_ledger.round_winner_wallet = participant.wallet;
                finalization_ledger.round_winner_tickets = participant.tickets_bought;
                finalization_ledger.round_winner_found = true;
            }
            finalization_ledger.round_remaining_tickets_seen = end;
        }

        batch_size = batch_size.checked_add(1).ok_or(Error::MathOverflow)?;
    }

    if finalization_ledger.round_processed_count > finalization_ledger.eligible_count {
        return Err(Error::InvalidInstruction.into());
    }

    let mut completed = false;
    if finalization_ledger.round_processed_count == finalization_ledger.eligible_count {
        let expected_remaining_tickets = finalization_ledger.remaining_tickets()?;
        if finalization_ledger.round_remaining_tickets_seen != expected_remaining_tickets {
            return Err(Error::InvalidInstruction.into());
        }
        if !finalization_ledger.round_winner_found {
            return Err(Error::WinnerNotFound.into());
        }

        finalization_ledger.push_winner(
            finalization_ledger.round_winner_wallet,
            finalization_ledger.round_winner_tickets,
        )?;

        if finalization_ledger.winners.len() == finalization_ledger.target_winners as usize {
            complete_finalization(
                config,
                lottery_ai,
                lottery,
                finalization_ledger,
                current_time,
            )?;
            completed = true;
        } else {
            finalization_ledger.current_round = finalization_ledger
                .current_round
                .checked_add(1)
                .ok_or(Error::MathOverflow)?;
            let remaining_tickets = finalization_ledger.remaining_tickets()?;
            let draw_index = draw_index_rpd_v3(
                &finalization_ledger.seed,
                finalization_ledger.current_round,
                remaining_tickets,
            )?;
            finalization_ledger.begin_selection_round(draw_index);
        }
    }

    Ok((
        batch_size,
        finalization_ledger.round_processed_count,
        finalization_ledger.eligible_count,
        completed,
    ))
}

fn complete_finalization(
    config: &Config,
    lottery_ai: &AccountInfo,
    lottery: &mut Lottery,
    finalization_ledger: &mut FinalizationLedger,
    current_time: i64,
) -> Result<(), solana_program::program_error::ProgramError> {
    let service_fee = compute_service_fee(lottery.total_funds, config.service_charge_bps)?;
    let winners_pool = lottery.total_funds.saturating_sub(service_fee);
    let winners_len = finalization_ledger.winners.len();
    if winners_len == 0 {
        return Err(Error::WinnerNotFound.into());
    }

    let per_winner_payout = winners_pool / (winners_len as u64);
    let total_winners_payout = per_winner_payout
        .checked_mul(winners_len as u64)
        .ok_or(Error::MathOverflow)?;

    let winner_entries: Vec<WinnerEntry> = finalization_ledger
        .winners
        .iter()
        .enumerate()
        .map(|(index, winner)| WinnerEntry {
            index: index as u64,
            recipient: winner.wallet,
            amount: per_winner_payout,
        })
        .collect();
    let merkle_root = build_winners_merkle_tree(&winner_entries);

    lottery.initialize_settlement(merkle_root, winners_len as u64, total_winners_payout)?;
    if lottery.settlement_start_unix == 0 {
        lottery.settlement_start_unix = current_time;
    }
    finalization_ledger.complete(current_time);

    let seed_hex = bytes_to_hex(&finalization_ledger.seed);
    let winner_wallets: Vec<String> = finalization_ledger
        .winners
        .iter()
        .map(|winner| winner.wallet.to_string())
        .collect();

    LotteryEvent::WinnersComputed {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        seed: seed_hex,
        rule_version: WINNER_ALGO_RULE_VERSION.to_string(),
        total_eligible: finalization_ledger.eligible_count,
        winners: winner_wallets.clone(),
        timestamp: current_time,
    }
    .emit();

    LotteryEvent::WinnersFinalized {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        winners_count: winners_len as u64,
        total_payout: total_winners_payout,
        per_winner_payout,
        winners_merkle_root: merkle_root,
        winners: winner_wallets,
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

fn require_sorted_wallet(
    finalization_ledger: &mut FinalizationLedger,
    wallet: &Pubkey,
) -> Result<(), Error> {
    if finalization_ledger.has_last_wallet
        && wallet.to_bytes() <= finalization_ledger.last_processed_wallet.to_bytes()
    {
        return Err(Error::InvalidInstruction);
    }
    finalization_ledger.last_processed_wallet = *wallet;
    finalization_ledger.has_last_wallet = true;
    Ok(())
}

fn extend_participants_commitment(
    current_commitment: [u8; 32],
    wallet: &Pubkey,
    tickets: u64,
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(RPD_V3_POOL_DOMAIN);
    h.update(current_commitment);
    h.update(wallet.to_bytes());
    h.update(tickets.to_le_bytes());
    let mut result = [0u8; 32];
    result.copy_from_slice(&h.finalize());
    result
}

fn compute_seed_rpd_v3(
    lottery_id: u64,
    eligible_count: u64,
    total_eligible_tickets: u64,
    poc_aggregate_hash: [u8; 32],
    participants_commitment: [u8; 32],
) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(RPD_V3_SEED_DOMAIN);
    h.update(lottery_id.to_le_bytes());
    h.update(eligible_count.to_le_bytes());
    h.update(total_eligible_tickets.to_le_bytes());
    h.update(poc_aggregate_hash);
    h.update(participants_commitment);
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&h.finalize());
    seed
}

fn draw_index_rpd_v3(
    seed: &[u8; 32],
    round: u64,
    total_remaining_tickets: u64,
) -> Result<u64, Error> {
    if total_remaining_tickets == 0 {
        return Err(Error::InvalidInstruction);
    }

    let mut h = Sha256::new();
    h.update(RPD_V3_DRAW_DOMAIN);
    h.update(seed);
    h.update(round.to_le_bytes());
    let digest = h.finalize();

    let mut first_16 = [0u8; 16];
    first_16.copy_from_slice(&digest[..16]);
    let rand = u128::from_le_bytes(first_16);
    Ok((rand % (total_remaining_tickets as u128)) as u64)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut out, "{:02x}", byte);
    }
    out
}

#[derive(Clone)]
struct WinnerEntry {
    index: u64,
    recipient: Pubkey,
    amount: u64,
}

fn build_winners_merkle_tree(winners: &[WinnerEntry]) -> [u8; 32] {
    if winners.is_empty() {
        return [0; 32];
    }

    let mut leaves: Vec<[u8; 32]> = winners
        .iter()
        .map(|w| {
            let mut data = Vec::new();
            data.extend_from_slice(&w.index.to_le_bytes());
            data.extend_from_slice(&w.recipient.to_bytes());
            data.extend_from_slice(&w.amount.to_le_bytes());

            let hash = Sha256::digest(&data);
            let mut result = [0u8; 32];
            result.copy_from_slice(&hash);
            result
        })
        .collect();

    while leaves.len() > 1 {
        let mut next_level = Vec::new();
        for chunk in leaves.chunks(2) {
            let hash = if chunk.len() == 2 {
                let mut data = Vec::new();
                data.extend_from_slice(&chunk[0]);
                data.extend_from_slice(&chunk[1]);
                Sha256::digest(&data)
            } else {
                let mut data = Vec::new();
                data.extend_from_slice(&chunk[0]);
                data.extend_from_slice(&chunk[0]);
                Sha256::digest(&data)
            };
            let mut result = [0u8; 32];
            result.copy_from_slice(&hash);
            next_level.push(result);
        }
        leaves = next_level;
    }

    leaves[0]
}

fn phase_name(phase: u8) -> &'static str {
    match phase {
        FINALIZATION_PHASE_AGGREGATING => "aggregating",
        FINALIZATION_PHASE_SELECTING => "selecting",
        FINALIZATION_PHASE_COMPLETED => "completed",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;

    #[test]
    fn participant_commitment_depends_on_wallet_order() {
        let wallet_a = Pubkey::new_unique();
        let wallet_b = Pubkey::new_unique();

        let ab = extend_participants_commitment(
            extend_participants_commitment([0; 32], &wallet_a, 2),
            &wallet_b,
            3,
        );
        let ba = extend_participants_commitment(
            extend_participants_commitment([0; 32], &wallet_b, 3),
            &wallet_a,
            2,
        );

        assert_ne!(ab, ba);
    }

    #[test]
    fn seed_and_draw_are_deterministic() {
        let seed = compute_seed_rpd_v3(42, 2, 5, [7; 32], [9; 32]);
        assert_eq!(seed, compute_seed_rpd_v3(42, 2, 5, [7; 32], [9; 32]));
        assert_eq!(
            draw_index_rpd_v3(&seed, 0, 5).unwrap(),
            draw_index_rpd_v3(&seed, 0, 5).unwrap()
        );
    }

    #[test]
    fn sorted_wallet_cursor_rejects_duplicates_and_rewind() {
        let lottery = Pubkey::new_unique();
        let mut ledger = FinalizationLedger::new(lottery, 4, 100);
        let wallet_a = Pubkey::new_from_array([1; 32]);
        let wallet_b = Pubkey::new_from_array([2; 32]);

        require_sorted_wallet(&mut ledger, &wallet_a).unwrap();
        require_sorted_wallet(&mut ledger, &wallet_b).unwrap();
        assert!(require_sorted_wallet(&mut ledger, &wallet_b).is_err());
        assert!(require_sorted_wallet(&mut ledger, &wallet_a).is_err());
    }
}
