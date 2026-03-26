//! # Begin Reveal Phase Instruction (testing only)
//!
//! Allows the authority to force-open the reveal window immediately.
//! This is intended for local development and testing convenience and
//! may be removed in production builds.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery},
    utils::{
        account::{read_account_data, write_account_data},
        crypto::aggregate_hashes,
        pda::{assert_pda_owned, derive_lottery_pda},
        validation::{require_key_match, require_signer},
    },
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::Sysvar,
};

/// Process the BeginRevealNow instruction (duration-based)
///
/// Authority-only. Sets `reveal_start_unix` to now and `reveal_deadline_unix`
/// to the standard duration from now (24h in production, 10 minutes with
/// `short_time`). Fails if the lottery is already settled or if we're already
/// in the reveal window.
///
/// # Accounts Expected
/// 0. `[]` Config account
/// 1. `[writable]` Lottery account
/// 2. `[signer]` Authority
pub fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    attestation_secs: u32,
    upload_secs: u32,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let authority_ai = next_account_info(account_info_iter)?;

    // Validate PDAs and signer
    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_signer(authority_ai)?;

    // Load accounts
    let config: Config = read_account_data(config_ai)?;
    let mut lottery: Lottery = read_account_data(lottery_ai)?;

    // Validate lottery PDA by deriving from stored state
    let (expected_lottery, _bump) = derive_lottery_pda(program_id, &lottery.config, lottery.id);
    require_key_match(lottery_ai, &expected_lottery)?;

    // Authority check
    if authority_ai.key != &config.authority {
        return Err(Error::Unauthorized.into());
    }

    // Must not be settled
    if lottery.settled {
        return Err(Error::LotteryAlreadySettled.into());
    }

    // Compute from chain clock, using config defaults when params are zero
    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let current_phase = lottery.phase(now);
    let already_started = lottery.upload_start_unix > 0 && now >= lottery.upload_start_unix;
    let already_ended = lottery.upload_deadline_unix > 0 && now > lottery.upload_deadline_unix;
    let settlement_started = lottery.settlement_start_unix > 0;
    let uploads_complete = lottery.uploads_complete || lottery.settlement_complete;
    if already_started || already_ended || settlement_started || uploads_complete {
        solana_program::msg!(
            "Invalid phase transition: BeginRevealNow lottery_id={} current_phase={} now={} upload_start={} upload_deadline={} settlement_start={} uploads_complete={} settlement_complete={} settled={}",
            lottery.id,
            current_phase,
            now,
            lottery.upload_start_unix,
            lottery.upload_deadline_unix,
            lottery.settlement_start_unix,
            lottery.uploads_complete,
            lottery.settlement_complete,
            lottery.settled
        );
        return Err(Error::InvalidPhaseTransition.into());
    }
    let att = if attestation_secs == 0 {
        config.upload_window_secs as i64
    } else {
        attestation_secs as i64
    };
    let upl = if upload_secs == 0 {
        config.upload_window_secs as i64
    } else {
        upload_secs as i64
    };

    // Start or extend windows; never shorten existing ones
    if lottery.upload_start_unix == 0 || now < lottery.upload_start_unix {
        lottery.upload_start_unix = now;
    }
    let desired_deadline = now + att;
    if desired_deadline > lottery.upload_deadline_unix {
        lottery.upload_deadline_unix = desired_deadline;
    }
    let desired_upload = lottery.upload_deadline_unix + upl;
    if desired_upload > lottery.upload_deadline_unix {
        lottery.upload_deadline_unix = desired_upload;
    }

    // If there is only one participant at the start of attestation, auto-complete uploads
    // and set selected winners to 1. This allows immediate settlement without off-chain upload.
    if lottery.participants_count == 1 {
        let empty_hashes: Vec<[u8; 32]> = Vec::new();
        let aggregate_hash = aggregate_hashes(&empty_hashes);
        lottery.mark_uploads_complete(aggregate_hash);
        // Safe: with one participant we want exactly one winner
        // set_selected_winners will currently reject participants_count<=1; bypass with direct set
        lottery.selected_number_of_winners = 1;

        // Persist this change before emitting event
        write_account_data(lottery_ai, "Lottery", &lottery)?;

        LotteryEvent::RevealsUploaded {
            lottery_id: lottery.id,
            lottery: lottery_ai.key.to_string(),
            authority: authority_ai.key.to_string(),
            participants_count: 1,
            aggregate_hash,
            selected_number_of_winners: 1,
            timestamp: now,
        }
        .emit();
    }

    // Persist
    write_account_data(lottery_ai, "Lottery", &lottery)?;

    // Emit explicit event for attestation phase begin
    LotteryEvent::UploadPhaseBegan {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        new_start: now,
        new_deadline: lottery.upload_deadline_unix,
        timestamp: now,
    }
    .emit();

    Ok(())
}
