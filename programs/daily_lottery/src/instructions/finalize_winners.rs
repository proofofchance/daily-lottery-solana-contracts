//! # Finalize Winners Instruction
//!
//! This instruction computes the lottery winners deterministically and stores
//! the winners merkle root on-chain. This is the first step of settlement.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery, Participant, Vault},
    utils::{
        account::{read_account_data, write_account_data},
        pda::assert_pda_owned,
        validation::{compute_service_fee, require_signer, require_writable},
    },
};
use sha2::{Digest, Sha256};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::Sysvar,
};
use std::{collections::HashSet, fmt::Write as _};

pub const WINNER_ALGO_RULE_VERSION: &str = "reveal-plaintext-draw-v2";
const RPD_V2_SEED_DOMAIN: &[u8] = &[
    0x49, 0x4b, 0x49, 0x47, 0x41, 0x49, 0x5f, 0x52, 0x50, 0x44, 0x5f, 0x56, 0x32, 0x5f, 0x53,
    0x45, 0x45, 0x44,
];
const RPD_V2_DRAW_DOMAIN: &[u8] = &[
    0x49, 0x4b, 0x49, 0x47, 0x41, 0x49, 0x5f, 0x52, 0x50, 0x44, 0x5f, 0x56, 0x32, 0x5f, 0x44,
    0x52, 0x41, 0x57,
];

type DrawPoolEntry = (Pubkey, u64);

/// Finalize winners and store commitment on-chain.
pub fn process(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let vault_ai = next_account_info(account_info_iter)?;
    let authority_ai = next_account_info(account_info_iter)?;

    let participant_accounts: Vec<&AccountInfo> = account_info_iter.collect();

    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(lottery_ai)?;
    require_writable(vault_ai)?;
    require_signer(authority_ai)?;

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
        // Single-participant rounds settle through refund semantics.
        return Err(Error::InvalidInstruction.into());
    }

    if lottery.attested_count == 0 {
        // Policy: no-attester rounds must use FinalizeNoAttesters (refund-only).
        return Err(Error::NoAttestedParticipants.into());
    }

    let clock = solana_program::clock::Clock::get()?;
    let current_time = clock.unix_timestamp;
    let current_phase = lottery.phase(current_time);
    let upload_elapsed =
        lottery.upload_deadline_unix > 0 && current_time > lottery.upload_deadline_unix;

    if current_phase != "settlement" && current_phase != "settled" && !upload_elapsed {
        return Err(Error::InvalidInstruction.into());
    }

    // If upload batches are incomplete, we allow finalize only once upload deadline elapsed.
    if !lottery.uploads_complete && !upload_elapsed {
        return Err(Error::InvalidInstruction.into());
    }

    if lottery.total_tickets == 0 {
        lottery.settle();
        write_account_data(lottery_ai, "Lottery", &lottery)?;
        LotteryEvent::NoBuyersConcluded {
            lottery_id: lottery.id,
            lottery: lottery_ai.key.to_string(),
            timestamp: clock.unix_timestamp,
        }
        .emit();
        return Ok(());
    }

    let expected_participants = lottery.participants_count as usize;
    if participant_accounts.len() != expected_participants {
        return Err(Error::MissingAccount.into());
    }

    let mut all_participants = Vec::with_capacity(expected_participants);
    let mut seen_accounts: HashSet<Pubkey> = HashSet::new();
    for participant_ai in participant_accounts.iter() {
        require_writable(participant_ai)?;
        let participant: Participant = read_account_data(participant_ai)?;
        if !seen_accounts.insert(*participant_ai.key) {
            return Err(Error::InvalidInstruction.into());
        }
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
        all_participants.push(participant);
    }

    let voted_winner_count =
        compute_winner_count_from_attesters(&all_participants, lottery.participants_count);
    if voted_winner_count == 0 || voted_winner_count as usize > crate::state::sizes::MAX_WINNERS {
        return Err(Error::InvalidInstruction.into());
    }
    lottery.set_selected_winners(voted_winner_count)?;

    // Eligible winner pool is reveal-included participants with tickets.
    let mut pool: Vec<DrawPoolEntry> = all_participants
        .iter()
        .filter(|p| p.tickets_bought > 0 && p.reveal_included())
        .map(|p| (p.wallet, p.tickets_bought))
        .collect();
    pool.sort_by(|a, b| a.0.to_bytes().cmp(&b.0.to_bytes()));

    if pool.is_empty() {
        return Err(Error::WinnerNotFound.into());
    }

    let total_revealed_tickets: u64 = pool.iter().map(|(_, tickets)| *tickets).sum();
    if total_revealed_tickets == 0 {
        return Err(Error::WinnerNotFound.into());
    }

    let seed = compute_seed_rpd_v2(
        lottery.id,
        pool.len() as u64,
        total_revealed_tickets,
        lottery.poc_aggregate_hash,
        &pool,
    );
    let winners_target = usize::min(voted_winner_count as usize, pool.len());
    let winners = select_winners_rpd_v2(&seed, &pool, winners_target)?;
    if winners.is_empty() {
        return Err(Error::WinnerNotFound.into());
    }

    let service_fee = compute_service_fee(lottery.total_funds, config.service_charge_bps)?;
    let winners_payout_total = lottery.total_funds.saturating_sub(service_fee);
    let per_winner_payout = if winners_payout_total == 0 {
        0
    } else {
        winners_payout_total / (winners.len() as u64)
    };

    let winner_entries: Vec<WinnerEntry> = winners
        .iter()
        .enumerate()
        .map(|(index, wallet)| WinnerEntry {
            index: index as u64,
            recipient: *wallet,
            amount: per_winner_payout,
        })
        .collect();
    let merkle_root = build_winners_merkle_tree(&winner_entries);

    lottery.initialize_settlement(merkle_root, winners.len() as u64, winners_payout_total)?;
    if lottery.settlement_start_unix == 0 {
        lottery.settlement_start_unix = clock.unix_timestamp;
    }

    let required_size = crate::state::sizes::LOTTERY_SIZE;
    if lottery_ai.data.borrow().len() < required_size {
        solana_program::msg!(
            "FINALIZE ERROR: Lottery account too small. Required: {}, Actual: {}.",
            required_size,
            lottery_ai.data.borrow().len()
        );
        return Err(Error::InvalidInstruction.into());
    }
    write_account_data(lottery_ai, "Lottery", &lottery)?;

    let seed_hex = bytes_to_hex(&seed);
    let winner_wallets: Vec<String> = winners.iter().map(|w| w.to_string()).collect();

    LotteryEvent::WinnersComputed {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        seed: seed_hex,
        rule_version: WINNER_ALGO_RULE_VERSION.to_string(),
        total_eligible: pool.len() as u64,
        winners: winner_wallets.clone(),
        timestamp: clock.unix_timestamp,
    }
    .emit();

    LotteryEvent::WinnersFinalized {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        winners_count: winners.len() as u64,
        total_payout: winners_payout_total,
        per_winner_payout,
        winners_merkle_root: merkle_root,
        winners: winner_wallets,
        timestamp: clock.unix_timestamp,
    }
    .emit();

    Ok(())
}

fn compute_winner_count_from_attesters(
    participants: &[Participant],
    participants_count: u64,
) -> u64 {
    if participants_count <= 1 {
        return 1;
    }

    let max_count = (crate::state::sizes::MAX_WINNERS as u64).min(participants_count - 1);
    let mut weights = vec![0u128; (max_count as usize).saturating_add(1)];
    let mut first_seen = vec![i64::MAX; (max_count as usize).saturating_add(1)];

    for participant in participants.iter() {
        if !participant.attested_uploaded || participant.tickets_bought == 0 {
            continue;
        }

        let voted = participant.voted_winners();
        if voted == 0 || voted > max_count {
            continue;
        }

        let idx = voted as usize;
        let weight = participant.tickets_bought as u128;
        weights[idx] = weights[idx].saturating_add(weight);
        if participant.attested_at_unix < first_seen[idx] {
            first_seen[idx] = participant.attested_at_unix;
        }
    }

    let mut best_count = 1u64;
    let mut best_weight = 0u128;
    let mut best_time = i64::MAX;
    for count in 1..=max_count {
        let idx = count as usize;
        let weight = weights[idx];
        if weight == 0 {
            continue;
        }
        let time = first_seen[idx];
        if weight > best_weight
            || (weight == best_weight && time < best_time)
            || (weight == best_weight && time == best_time && count < best_count)
        {
            best_weight = weight;
            best_time = time;
            best_count = count;
        }
    }

    if best_weight == 0 {
        1
    } else {
        best_count
    }
}

fn compute_seed_rpd_v2(
    lottery_id: u64,
    participants_count: u64,
    total_tickets: u64,
    poc_aggregate_hash: [u8; 32],
    participants_sorted: &[DrawPoolEntry],
) -> [u8; 32] {
    let mut h0 = Sha256::new();
    h0.update(RPD_V2_SEED_DOMAIN);
    h0.update(lottery_id.to_le_bytes());
    h0.update(participants_count.to_le_bytes());
    h0.update(total_tickets.to_le_bytes());
    h0.update(poc_aggregate_hash);
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&h0.finalize());

    for (wallet, tickets) in participants_sorted.iter() {
        let mut h = Sha256::new();
        h.update(seed);
        h.update(wallet.to_bytes());
        h.update(tickets.to_le_bytes());
        seed.copy_from_slice(&h.finalize());
    }

    seed
}

fn draw_index_rpd_v2(
    seed: &[u8; 32],
    round: u64,
    total_remaining_tickets: u64,
) -> Result<u64, Error> {
    if total_remaining_tickets == 0 {
        return Err(Error::InvalidInstruction);
    }

    let mut h = Sha256::new();
    h.update(RPD_V2_DRAW_DOMAIN);
    h.update(seed);
    h.update(round.to_le_bytes());
    let digest = h.finalize();

    let mut first_16 = [0u8; 16];
    first_16.copy_from_slice(&digest[..16]);
    let rand = u128::from_le_bytes(first_16);
    Ok((rand % (total_remaining_tickets as u128)) as u64)
}

fn select_weighted_pos(pool: &[DrawPoolEntry], ticket_index: u64) -> Option<usize> {
    let mut cumulative = 0u64;
    for (idx, (_wallet, tickets)) in pool.iter().enumerate() {
        let end = cumulative.saturating_add(*tickets);
        if ticket_index < end {
            return Some(idx);
        }
        cumulative = end;
    }
    None
}

fn select_winners_rpd_v2(
    seed: &[u8; 32],
    pool_sorted: &[DrawPoolEntry],
    winners_to_select: usize,
) -> Result<Vec<Pubkey>, Error> {
    let mut pool = pool_sorted.to_vec();
    let mut winners: Vec<Pubkey> = Vec::new();

    for round in 0..winners_to_select.min(pool.len()) {
        let total_remaining: u64 = pool.iter().map(|(_, tickets)| *tickets).sum();
        if total_remaining == 0 {
            break;
        }

        let draw_index = draw_index_rpd_v2(seed, round as u64, total_remaining)?;
        let selected_pos = select_weighted_pos(&pool, draw_index).ok_or(Error::WinnerNotFound)?;
        let (winner_wallet, _tickets) = pool.remove(selected_pos);
        winners.push(winner_wallet);
    }

    Ok(winners)
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

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;

    fn sample_pool() -> Vec<DrawPoolEntry> {
        let mut pool = vec![
            (Pubkey::new_unique(), 5),
            (Pubkey::new_unique(), 2),
            (Pubkey::new_unique(), 3),
        ];
        pool.sort_by(|a, b| a.0.to_bytes().cmp(&b.0.to_bytes()));
        pool
    }

    #[test]
    fn seed_is_deterministic_for_same_inputs() {
        let pool = sample_pool();
        let s1 = compute_seed_rpd_v2(42, 3, 10, [7u8; 32], &pool);
        let s2 = compute_seed_rpd_v2(42, 3, 10, [7u8; 32], &pool);
        assert_eq!(s1, s2);
    }

    #[test]
    fn seed_changes_when_inputs_change() {
        let pool = sample_pool();
        let base = compute_seed_rpd_v2(42, 3, 10, [7u8; 32], &pool);

        let mut changed_ticket = pool.clone();
        changed_ticket[0].1 += 1;
        let ticket_seed = compute_seed_rpd_v2(42, 3, 10, [7u8; 32], &changed_ticket);
        assert_ne!(base, ticket_seed);

        let changed_aggregate = compute_seed_rpd_v2(42, 3, 10, [8u8; 32], &pool);
        assert_ne!(base, changed_aggregate);

        let changed_lottery_id = compute_seed_rpd_v2(43, 3, 10, [7u8; 32], &pool);
        assert_ne!(base, changed_lottery_id);
    }

    #[test]
    fn draws_are_deterministic() {
        let pool = sample_pool();
        let seed = compute_seed_rpd_v2(42, 3, 10, [7u8; 32], &pool);

        let w1 = select_winners_rpd_v2(&seed, &pool, 2).unwrap();
        let w2 = select_winners_rpd_v2(&seed, &pool, 2).unwrap();
        assert_eq!(w1, w2);
    }

    #[test]
    fn winners_are_without_replacement() {
        let pool = sample_pool();
        let seed = compute_seed_rpd_v2(42, 3, 10, [7u8; 32], &pool);
        let winners = select_winners_rpd_v2(&seed, &pool, 3).unwrap();

        assert_eq!(winners.len(), 3);
        let unique: std::collections::HashSet<Pubkey> = winners.iter().copied().collect();
        assert_eq!(unique.len(), winners.len());
    }

    #[test]
    fn winner_count_uses_cumulative_weight() {
        let p1 = Participant {
            wallet: Pubkey::new_unique(),
            tickets_bought: 5,
            attested_uploaded: true,
            attested_at_unix: 100,
            voted_number_of_winners: 1,
            ..Participant::default()
        };
        let p2 = Participant {
            wallet: Pubkey::new_unique(),
            tickets_bought: 4,
            attested_uploaded: true,
            attested_at_unix: 50,
            voted_number_of_winners: 1,
            ..Participant::default()
        };
        let p3 = Participant {
            wallet: Pubkey::new_unique(),
            tickets_bought: 7,
            attested_uploaded: true,
            attested_at_unix: 10,
            voted_number_of_winners: 2,
            ..Participant::default()
        };

        let selected = compute_winner_count_from_attesters(&[p1, p2, p3], 4);
        assert_eq!(selected, 1);
    }

    #[test]
    fn winner_count_tie_breaks_by_earliest_attestation_then_lower_count() {
        let p1 = Participant {
            wallet: Pubkey::new_unique(),
            tickets_bought: 5,
            attested_uploaded: true,
            attested_at_unix: 10,
            voted_number_of_winners: 1,
            ..Participant::default()
        };
        let p2 = Participant {
            wallet: Pubkey::new_unique(),
            tickets_bought: 5,
            attested_uploaded: true,
            attested_at_unix: 20,
            voted_number_of_winners: 2,
            ..Participant::default()
        };

        let selected = compute_winner_count_from_attesters(&[p1.clone(), p2], 5);
        assert_eq!(selected, 1);

        let p3 = Participant {
            wallet: Pubkey::new_unique(),
            tickets_bought: 5,
            attested_uploaded: true,
            attested_at_unix: 10,
            voted_number_of_winners: 2,
            ..Participant::default()
        };
        let selected_same_time = compute_winner_count_from_attesters(&[p1, p3], 5);
        assert_eq!(selected_same_time, 1);
    }
}
