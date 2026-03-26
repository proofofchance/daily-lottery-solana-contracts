//! # Begin Upload Phase Instruction
//!
//! Authority-only. Sets `upload_start_unix` to now and `upload_deadline_unix`
//! to now + (param or config.upload_window_secs). Uses chain clock.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery},
    utils::{
        account::{read_account_data, write_account_data},
        crypto::aggregate_hashes,
        pda::{assert_pda_owned, derive_lottery_pda},
        validation::{require_key_match, require_signer, require_writable},
    },
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::Sysvar,
};

pub fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    upload_secs: u32,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let authority_ai = next_account_info(account_info_iter)?;

    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_signer(authority_ai)?;
    require_writable(lottery_ai)?;

    let config: Config = read_account_data(config_ai)?;
    let mut lottery: Lottery = read_account_data(lottery_ai)?;

    // Validate lottery PDA by deriving from stored state
    let (expected_lottery, _bump) = derive_lottery_pda(program_id, &lottery.config, lottery.id);
    require_key_match(lottery_ai, &expected_lottery)?;

    // Authority check
    if authority_ai.key != &config.authority {
        return Err(Error::Unauthorized.into());
    }

    if lottery.settled {
        return Err(Error::LotteryAlreadySettled.into());
    }

    // Must be after buy deadline
    let now = Clock::get()?.unix_timestamp;
    if lottery.buy_deadline_unix == 0 || now < lottery.buy_deadline_unix {
        return Err(Error::InvalidInstruction.into());
    }

    let current_phase = lottery.phase(now);
    let already_started = lottery.upload_start_unix > 0 && now >= lottery.upload_start_unix;
    let already_ended = lottery.upload_deadline_unix > 0 && now > lottery.upload_deadline_unix;
    let settlement_started = lottery.settlement_start_unix > 0;
    let uploads_complete = lottery.uploads_complete || lottery.settlement_complete;
    if already_started || already_ended || settlement_started || uploads_complete {
        solana_program::msg!(
            "Invalid phase transition: BeginUploadPhase lottery_id={} current_phase={} now={} upload_start={} upload_deadline={} settlement_start={} uploads_complete={} settlement_complete={} settled={}",
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

    // Compute upload window
    let ul = if upload_secs == 0 {
        config.upload_window_secs as i64
    } else {
        upload_secs as i64
    };

    if lottery.upload_start_unix == 0 || now < lottery.upload_start_unix {
        lottery.upload_start_unix = now;
    }
    let desired_deadline = now + ul;
    if desired_deadline > lottery.upload_deadline_unix {
        lottery.upload_deadline_unix = desired_deadline;
    }

    // Single participant lotteries can skip off-chain uploads entirely.
    // Mark uploads complete and emit RevealsUploaded so indexers treat the lottery as upload-ready.
    if lottery.participants_count == 1 && !lottery.uploads_complete {
        let empty_hashes: Vec<[u8; 32]> = Vec::new();
        let aggregate_hash = aggregate_hashes(&empty_hashes);
        lottery.mark_uploads_complete(aggregate_hash);
        lottery.selected_number_of_winners = 1;

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

    write_account_data(lottery_ai, "Lottery", &lottery)?;

    LotteryEvent::UploadPhaseBegan {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        new_start: lottery.upload_start_unix,
        new_deadline: lottery.upload_deadline_unix,
        timestamp: now,
    }
    .emit();

    Ok(())
}
