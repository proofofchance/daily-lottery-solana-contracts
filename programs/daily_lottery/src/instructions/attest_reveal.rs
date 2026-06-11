//! # Permissionless Reveal Attestation Instruction
//!
//! Lets a participant attest and include their reveal on-chain without a provider
//! receipt. This gives participants a protocol-level escape hatch if the provider
//! does not sign an off-chain upload receipt.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery, Participant},
    utils::{
        account::{read_account_data, write_account_data},
        crypto::{compute_reveal_digest, xor_reveal_digests},
        limits::MAX_REVEAL_PLAINTEXT_BYTES,
        pda::assert_pda_owned,
        validation::{require_signer, require_writable},
    },
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::Sysvar,
};
use solana_sha256_hasher::hash;

/// Process an on-chain reveal attestation.
///
/// Accounts expected:
/// 0. `[]` Config account
/// 1. `[writable]` Lottery account
/// 2. `[writable]` Participant account
/// 3. `[signer]` Participant wallet
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

    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(lottery_ai)?;
    require_writable(participant_ai)?;
    require_signer(wallet_ai)?;

    if reveal_plaintext.is_empty() || reveal_plaintext.len() > MAX_REVEAL_PLAINTEXT_BYTES {
        return Err(Error::InvalidUploads.into());
    }

    let _config: Config = read_account_data(config_ai)?;
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
    let max_winners = if lottery.participants_count <= 1 {
        1
    } else {
        lottery.participants_count.saturating_sub(1)
    };
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
    participant.mark_reveal_included();

    lottery.add_attestation()?;
    lottery.provider_uploaded_count = lottery.provider_uploaded_count.saturating_add(1);
    lottery.poc_aggregate_hash = aggregate_hash;

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
