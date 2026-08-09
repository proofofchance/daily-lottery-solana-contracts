//! # Permissionless Reveal Attestation Instruction
//!
//! Lets a participant attest and include their reveal on-chain without a provider
//! receipt. This gives participants a protocol-level escape hatch if the provider
//! does not sign an off-chain upload receipt.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery, Participant, VoteTally},
    utils::{
        account::{read_account_data, write_account_data},
        crypto::{compute_reveal_digest, xor_reveal_digests},
        limits::MAX_REVEAL_PLAINTEXT_BYTES,
        pda::{assert_pda_key, assert_pda_owned, derive_vote_tally_pda},
        validation::{require_key_match, require_signer, require_writable},
    },
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    program::invoke_signed,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::Sysvar,
};
use solana_sha256_hasher::hash;
use solana_system_interface::{instruction as system_instruction, program as system_program};

/// Process an on-chain reveal attestation.
///
/// Accounts expected:
/// 0. `[]` Config account
/// 1. `[writable]` Lottery account
/// 2. `[writable]` Participant account
/// 3. `[signer, writable]` Participant wallet
/// 4. `[writable]` VoteTally PDA (`["vote_tally", lottery]`)
/// 5. `[]` System program
pub fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    voted_number_of_winners: u64,
    reveal_plaintext: Vec<u8>,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let participant_ai = next_account_info(account_info_iter)?;
    let wallet_ai = next_account_info(account_info_iter)?;
    let vote_tally_ai = next_account_info(account_info_iter)?;
    let system_program_ai = next_account_info(account_info_iter)?;

    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(lottery_ai)?;
    require_writable(participant_ai)?;
    require_signer(wallet_ai)?;
    require_writable(wallet_ai)?;
    require_writable(vote_tally_ai)?;
    require_key_match(system_program_ai, &system_program::id())?;

    if reveal_plaintext.is_empty() || reveal_plaintext.len() > MAX_REVEAL_PLAINTEXT_BYTES {
        return Err(Error::InvalidUploads.into());
    }

    let config: Config = read_account_data(config_ai)?;
    let mut lottery: Lottery = read_account_data(lottery_ai)?;
    let mut participant: Participant = read_account_data(participant_ai)?;

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

    assert_pda_owned(
        program_id,
        participant_ai,
        &[
            b"participant",
            lottery_ai.key.as_ref(),
            wallet_ai.key.as_ref(),
        ],
    )?;
    if participant.lottery != *lottery_ai.key || participant.wallet != *wallet_ai.key {
        return Err(Error::InvalidAccountData.into());
    }
    if !participant.has_tickets() {
        return Err(Error::InvalidInstruction.into());
    }
    if participant.attested_uploaded {
        return Err(Error::AttestationAlreadySubmitted.into());
    }
    if participant.reveal_included() {
        return Err(Error::InvalidUploads.into());
    }

    let current_time = Clock::get()?.unix_timestamp;
    if lottery.phase(current_time) != "upload" {
        return Err(Error::OutsideTimeWindow.into());
    }

    if voted_number_of_winners == 0 {
        return Err(Error::InvalidInstruction.into());
    }
    let max_winners = config.effective_max_winners(lottery.participants_count);
    if voted_number_of_winners > max_winners {
        return Err(Error::InvalidInstruction.into());
    }

    if hash(&reveal_plaintext).to_bytes() != participant.proof_of_chance_hash {
        return Err(Error::RevealMismatch.into());
    }

    let reveal_digest = compute_reveal_digest(wallet_ai.key, &reveal_plaintext);
    let aggregate_hash = xor_reveal_digests(lottery.poc_aggregate_hash, &[reveal_digest]);

    participant.set_vote_number_of_winners(voted_number_of_winners)?;
    participant.attest_upload(current_time)?;
    participant.include_verified_reveal(reveal_digest);

    lottery.add_attestation()?;
    lottery.provider_uploaded_count = lottery.provider_uploaded_count.saturating_add(1);
    lottery.poc_aggregate_hash = aggregate_hash;

    let max_winners = config.effective_max_winners(lottery.participants_count);
    let (expected_vote_tally, bump) = derive_vote_tally_pda(program_id, lottery_ai.key);
    if expected_vote_tally != *vote_tally_ai.key {
        return Err(Error::InvalidSeeds.into());
    }
    let mut vote_tally = if vote_tally_ai.data_is_empty() {
        assert_pda_key(
            program_id,
            vote_tally_ai,
            &[b"vote_tally", lottery_ai.key.as_ref()],
        )?;
        let space = VoteTally::account_size_for(max_winners as usize);
        let lamports = Rent::get()?.minimum_balance(space);
        let create_ix = system_instruction::create_account(
            wallet_ai.key,
            vote_tally_ai.key,
            lamports,
            space as u64,
            program_id,
        );
        invoke_signed(
            &create_ix,
            &[
                wallet_ai.clone(),
                vote_tally_ai.clone(),
                system_program_ai.clone(),
            ],
            &[&[b"vote_tally", lottery_ai.key.as_ref(), &[bump]]],
        )?;
        VoteTally::new(*lottery_ai.key, max_winners, lottery.attested_count)
    } else {
        assert_pda_owned(
            program_id,
            vote_tally_ai,
            &[b"vote_tally", lottery_ai.key.as_ref()],
        )?;
        let tally: VoteTally = read_account_data(vote_tally_ai)?;
        if tally.lottery != *lottery_ai.key || tally.max_winners != max_winners {
            return Err(Error::InvalidAccountData.into());
        }
        tally
    };
    vote_tally.total_attested = lottery.attested_count;
    vote_tally.add_vote(
        participant.voted_winners(),
        participant.tickets_bought as u128,
        participant.attested_at_unix,
    );
    vote_tally.processed_count = vote_tally
        .processed_count
        .checked_add(1)
        .ok_or(Error::MathOverflow)?;
    if lottery.participants_count > 1 {
        lottery.set_selected_winners(vote_tally.selected_winners(lottery.participants_count))?;
    }

    let all_attested =
        lottery.participants_count > 0 && lottery.attested_count == lottery.participants_count;
    let upload_elapsed =
        lottery.upload_deadline_unix > 0 && current_time > lottery.upload_deadline_unix;
    if all_attested && lottery.settlement_start_unix == 0 {
        lottery.settlement_start_unix = current_time;
    }
    if lottery.provider_uploaded_count >= lottery.attested_count && (all_attested || upload_elapsed)
    {
        lottery.uploads_complete = true;
    }

    write_account_data(participant_ai, "Participant", &participant)?;
    write_account_data(lottery_ai, "Lottery", &lottery)?;
    write_account_data(vote_tally_ai, "VoteTally", &vote_tally)?;

    LotteryEvent::AttestationSubmitted {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        participant: participant_ai.key.to_string(),
        wallet: wallet_ai.key.to_string(),
        voted_number_of_winners,
        total_attested: lottery.attested_count,
        timestamp: current_time,
    }
    .emit();

    LotteryEvent::RevealsUploaded {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        authority: wallet_ai.key.to_string(),
        participants_count: lottery.provider_uploaded_count,
        aggregate_hash,
        selected_number_of_winners: lottery.selected_number_of_winners,
        timestamp: current_time,
    }
    .emit();

    Ok(())
}
