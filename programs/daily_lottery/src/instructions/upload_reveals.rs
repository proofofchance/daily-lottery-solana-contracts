//! # Upload Proofs (Chunk) Instruction
//!
//! Allows the service provider to upload batch reveals for settlement.
//! This instruction is called by the authority to upload all participant reveals
//! after the reveal window has opened and participants have attested.

use crate::{
    error::Error,
    state::{Config, Lottery, Participant, VoteTally},
    utils::{
        account::{read_account_data, validate_account_discriminator, write_account_data},
        limits::MAX_REVEAL_PLAINTEXT_BYTES,
        pda::{assert_pda_key, assert_pda_owned, derive_vote_tally_pda},
        validation::{require_key_match, require_signer, require_writable},
    },
};
use solana_program::sysvar::rent::Rent;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    program::invoke_signed,
    pubkey::Pubkey,
    sysvar::Sysvar,
};
use solana_sha256_hasher::{hash, hashv};
use solana_system_interface::{instruction as system_instruction, program as system_program};

const RPD_V2_REVEAL_DOMAIN: &[u8] = &[
    0x49, 0x4b, 0x49, 0x47, 0x41, 0x49, 0x5f, 0x52, 0x50, 0x44, 0x5f, 0x56, 0x32, 0x5f, 0x52,
    0x45, 0x56, 0x45, 0x41, 0x4c,
];

fn compute_reveal_digest(wallet: &Pubkey, plaintext: &[u8]) -> [u8; 32] {
    let plaintext_len_le = (plaintext.len() as u32).to_le_bytes();
    hashv(&[
        RPD_V2_REVEAL_DOMAIN,
        &wallet.to_bytes(),
        &plaintext_len_le,
        plaintext,
    ])
    .to_bytes()
}

fn xor_reveal_digests(initial: [u8; 32], digests: &[[u8; 32]]) -> [u8; 32] {
    let mut aggregate_hash = initial;
    for digest in digests.iter() {
        for i in 0..aggregate_hash.len() {
            aggregate_hash[i] ^= digest[i];
        }
    }
    aggregate_hash
}

/// Process the Upload Proofs (chunk) instruction
///
/// Uploads batch reveals from all attested participants and prepares for settlement.
/// Only the authority can call this instruction during the reveal window.
///
/// # Accounts Expected
/// 0. `[]` Config account
/// 1. `[writable]` Lottery account
/// 2. `[signer]` Authority (payer if VoteTally PDA is created)
/// 3. `[]` System program
/// 4. `[writable]` VoteTally PDA (`["vote_tally", lottery]`)
///    5..N. `[]` Participant accounts (for all attested participants)
pub fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    entries: Vec<(Pubkey, Vec<u8>)>,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    // Get accounts
    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let authority_ai = next_account_info(account_info_iter)?;
    let system_program_ai = next_account_info(account_info_iter)?;
    let vote_tally_ai = next_account_info(account_info_iter)?;

    // Validate accounts
    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(lottery_ai)?;
    require_signer(authority_ai)?;
    require_writable(vote_tally_ai)?;
    require_key_match(system_program_ai, &system_program::id())?;

    if !validate_account_discriminator(config_ai, "Config") {
        return Err(Error::InvalidAccountData.into());
    }
    if !validate_account_discriminator(lottery_ai, "Lottery") {
        return Err(Error::InvalidAccountData.into());
    }

    // Read account data
    let config: Config = read_account_data(config_ai)?;
    let mut lottery: Lottery = read_account_data(lottery_ai)?;

    assert_pda_owned(
        program_id,
        lottery_ai,
        &[
            b"lottery",
            config_ai.key.as_ref(),
            &lottery.id.to_le_bytes(),
        ],
    )?;
    if lottery.config != *config_ai.key {
        return Err(Error::InvalidAccountData.into());
    }

    // Validate authority
    if authority_ai.key != &config.authority {
        return Err(Error::Unauthorized.into());
    }

    // Get current time for event timestamp
    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;

    // Must not be settled; enforce monotonic transition into settlement
    if lottery.settled || lottery.settlement_complete || lottery.uploads_complete {
        solana_program::msg!(
            "Invalid phase transition: UploadReveals lottery_id={} now={} settlement_start={} uploads_complete={} settlement_complete={} settled={}",
            lottery.id,
            current_time,
            lottery.settlement_start_unix,
            lottery.uploads_complete,
            lottery.settlement_complete,
            lottery.settled
        );
        return Err(Error::InvalidPhaseTransition.into());
    }

    let upload_elapsed =
        lottery.upload_deadline_unix > 0 && current_time > lottery.upload_deadline_unix;
    let all_attested =
        lottery.participants_count > 0 && lottery.attested_count == lottery.participants_count;

    if !upload_elapsed && !all_attested && lottery.settlement_start_unix == 0 {
        solana_program::msg!(
            "Invalid phase transition: UploadReveals lottery_id={} now={} upload_deadline={} attested_count={} participants_count={} settlement_start={}",
            lottery.id,
            current_time,
            lottery.upload_deadline_unix,
            lottery.attested_count,
            lottery.participants_count,
            lottery.settlement_start_unix
        );
        return Err(Error::InvalidPhaseTransition.into());
    }

    if lottery.settlement_start_unix == 0 {
        lottery.settlement_start_unix = current_time;
    }

    // Ensure there are attested participants before creating vote tally or processing
    if lottery.attested_count == 0 {
        return Err(Error::NoAttestedParticipants.into());
    }

    // Initialize or load the VoteTally PDA (tracks vote weights across batches)
    let max_winners = if lottery.participants_count <= 1 {
        1u64
    } else {
        std::cmp::min(
            crate::state::sizes::MAX_WINNERS as u64,
            lottery.participants_count.saturating_sub(1),
        )
    };
    let (expected_vote_tally, bump) = derive_vote_tally_pda(program_id, lottery_ai.key);
    if expected_vote_tally != *vote_tally_ai.key {
        return Err(Error::InvalidSeeds.into());
    }
    let mut vote_tally = if vote_tally_ai.data_is_empty() {
        // Authority pays for account creation
        require_writable(authority_ai)?;
        assert_pda_key(
            program_id,
            vote_tally_ai,
            &[b"vote_tally", lottery_ai.key.as_ref()],
        )?;
        let space = VoteTally::account_size_for(max_winners as usize);
        let lamports = Rent::get()?.minimum_balance(space);
        let create_ix = system_instruction::create_account(
            authority_ai.key,
            vote_tally_ai.key,
            lamports,
            space as u64,
            program_id,
        );
        invoke_signed(
            &create_ix,
            &[
                authority_ai.clone(),
                vote_tally_ai.clone(),
                system_program_ai.clone(),
            ],
            &[&[b"vote_tally", lottery_ai.key.as_ref(), &[bump]]],
        )?;
        let tally = VoteTally::new(*lottery_ai.key, max_winners, lottery.attested_count);
        write_account_data(vote_tally_ai, "VoteTally", &tally)?;
        tally
    } else {
        assert_pda_owned(
            program_id,
            vote_tally_ai,
            &[b"vote_tally", lottery_ai.key.as_ref()],
        )?;
        let tally: VoteTally = read_account_data(vote_tally_ai)?;
        if tally.lottery != *lottery_ai.key {
            return Err(Error::InvalidInstruction.into());
        }
        if tally.max_winners != max_winners {
            return Err(Error::InvalidInstruction.into());
        }
        tally
    };

    // Build a map from participant -> plaintext, enforce no duplicates and length limits
    use std::collections::{BTreeMap, HashSet};
    let mut provided: BTreeMap<Pubkey, Vec<u8>> = BTreeMap::new();
    let mut seen: HashSet<Pubkey> = HashSet::new();
    for (pk, pt) in entries.into_iter() {
        if seen.contains(&pk) {
            return Err(Error::InvalidUploads.into());
        }
        if pt.len() > MAX_REVEAL_PLAINTEXT_BYTES {
            return Err(Error::InvalidUploads.into());
        }
        provided.insert(pk, pt);
        seen.insert(pk);
    }

    // Process each provided participant entry; verify PDA, attestation and hash, compute score.
    // Also build reveal-derived digests used by settlement entropy.
    let mut new_uploads = 0u64;
    let mut reveal_digests: Vec<[u8; 32]> = Vec::with_capacity(provided.len());

    for (participant_pk, plaintext) in provided.iter() {
        // Load participant account from remaining accounts by key match
        // For efficiency, we expect the account to be present in remaining accounts; otherwise fail
        let mut found_ai: Option<&AccountInfo> = None;
        for ai in account_info_iter.clone() {
            if ai.key == participant_pk {
                found_ai = Some(ai);
                break;
            }
        }
        let participant_ai = found_ai.ok_or(Error::MissingAccount)?;
        require_writable(participant_ai)?;
        let mut participant: Participant = read_account_data(participant_ai)?;

        // Verify the participant PDA matches the wallet stored in the account.
        assert_pda_owned(
            program_id,
            participant_ai,
            &[
                b"participant",
                lottery_ai.key.as_ref(),
                participant.wallet.as_ref(),
            ],
        )?;
        if participant.lottery != *lottery_ai.key {
            return Err(Error::InvalidAccountData.into());
        }

        if participant.reveal_included() {
            return Err(Error::InvalidUploads.into());
        }

        if !participant.attested_uploaded {
            return Err(Error::InvalidUploads.into());
        }

        // Verify PoC hash matches
        let reveal_hash = hash(plaintext);
        if reveal_hash.to_bytes() != participant.proof_of_chance_hash {
            return Err(Error::RevealMismatch.into());
        }

        // Settlement entropy input is reveal-derived and batch/order independent.
        let reveal_digest = compute_reveal_digest(&participant.wallet, plaintext);
        reveal_digests.push(reveal_digest);

        // Compute reveal_score from lucky words component for transparency analytics.
        // This is informational only and never used for winner settlement entropy.
        let sep = 0x1f;
        let lw_bytes = if let Some(pos) = plaintext.iter().position(|&b| b == sep) {
            &plaintext[..pos]
        } else {
            &plaintext[..]
        };
        let score: u64 = match core::str::from_utf8(lw_bytes) {
            Ok(s) => s.trim().to_ascii_lowercase().chars().count() as u64,
            Err(_) => {
                let mut start = 0usize;
                let mut end = lw_bytes.len();
                while start < end {
                    let b = lw_bytes[start];
                    if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                        start += 1;
                    } else {
                        break;
                    }
                }
                while end > start {
                    let b = lw_bytes[end - 1];
                    if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                        end -= 1;
                    } else {
                        break;
                    }
                }
                (end.saturating_sub(start)) as u64
            }
        };

        let voted = participant.voted_winners();
        if voted > 0 && voted <= lottery.participants_count.saturating_sub(1) {
            let weight = participant.tickets_bought as u128;
            vote_tally.add_vote(voted, weight, participant.attested_at_unix);
        }

        participant.reveal_score = score;
        participant.mark_reveal_included();
        write_account_data(participant_ai, "Participant", &participant)?;

        new_uploads = new_uploads.saturating_add(1);
    }

    // Note: chunked uploads do not require all attested in one call; we'll track progress
    vote_tally.processed_count = vote_tally.processed_count.saturating_add(new_uploads);

    // Aggregate reveal digests in a batch/order-independent way (XOR of digests).
    // This removes provider-controlled chunk partitioning bias.
    let aggregate_hash = xor_reveal_digests(lottery.poc_aggregate_hash, &reveal_digests);

    // Determine selected number of winners from cumulative vote tally
    let selected_winners_count = vote_tally.selected_winners(lottery.participants_count);

    // Update lottery state
    lottery.provider_uploaded_count = lottery.provider_uploaded_count.saturating_add(new_uploads);
    if lottery.provider_uploaded_count >= lottery.attested_count {
        lottery.uploads_complete = true;
    }
    lottery.poc_aggregate_hash = aggregate_hash;
    // Enforce bounds here too
    lottery.set_selected_winners(selected_winners_count)?;

    // Persist vote tally updates
    write_account_data(vote_tally_ai, "VoteTally", &vote_tally)?;

    // Emit event
    let event = crate::events::LotteryEvent::RevealsUploaded {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        authority: config.authority.to_string(),
        participants_count: lottery.provider_uploaded_count,
        aggregate_hash,
        selected_number_of_winners: lottery.selected_number_of_winners,
        timestamp: current_time,
    };
    event.emit();

    // Write updated lottery data
    write_account_data(lottery_ai, "Lottery", &lottery)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_digest_is_deterministic() {
        let wallet = Pubkey::new_unique();
        let plaintext = b"cool\x1f0123";
        let d1 = compute_reveal_digest(&wallet, plaintext);
        let d2 = compute_reveal_digest(&wallet, plaintext);
        assert_eq!(d1, d2);
    }

    #[test]
    fn reveal_digest_changes_with_wallet_or_plaintext() {
        let wallet_a = Pubkey::new_unique();
        let wallet_b = Pubkey::new_unique();
        let d1 = compute_reveal_digest(&wallet_a, b"hello\x1fabcd");
        let d2 = compute_reveal_digest(&wallet_a, b"hello!\x1fabcd");
        let d3 = compute_reveal_digest(&wallet_b, b"hello\x1fabcd");
        assert_ne!(d1, d2);
        assert_ne!(d1, d3);
    }

    #[test]
    fn xor_accumulator_is_order_independent() {
        let wallet_a = Pubkey::new_unique();
        let wallet_b = Pubkey::new_unique();
        let d1 = compute_reveal_digest(&wallet_a, b"one\x1faaaa");
        let d2 = compute_reveal_digest(&wallet_b, b"two\x1fbbbb");
        let left = xor_reveal_digests([0u8; 32], &[d1, d2]);
        let right = xor_reveal_digests([0u8; 32], &[d2, d1]);
        assert_eq!(left, right);
    }
}
