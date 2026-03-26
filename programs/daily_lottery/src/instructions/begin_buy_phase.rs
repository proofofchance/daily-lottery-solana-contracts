//! # Begin Buy Phase Instruction
//!
//! Authority-only. Sets `buy_start_unix` to now and `buy_deadline_unix`
//! to now + (param or config.buy_window_secs). Uses chain clock.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery},
    utils::{
        account::{read_account_data, write_account_data},
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

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], buy_secs: u32) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let authority_ai = next_account_info(account_info_iter)?;

    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_signer(authority_ai)?;
    require_writable(lottery_ai)?;

    let config: Config = read_account_data(config_ai)?;
    let mut lottery: Lottery = read_account_data(lottery_ai)?;

    let (expected_lottery, _bump) = derive_lottery_pda(program_id, &lottery.config, lottery.id);
    require_key_match(lottery_ai, &expected_lottery)?;

    if authority_ai.key != &config.authority {
        return Err(Error::Unauthorized.into());
    }

    if lottery.settled {
        return Err(Error::LotteryAlreadySettled.into());
    }

    let now = Clock::get()?.unix_timestamp;
    let current_phase = lottery.phase(now);
    let already_started = lottery.buy_start_unix > 0 && now >= lottery.buy_start_unix;
    let already_ended = lottery.buy_deadline_unix > 0 && now > lottery.buy_deadline_unix;
    let upload_started = lottery.upload_start_unix > 0 && now >= lottery.upload_start_unix;
    let settlement_started = lottery.settlement_start_unix > 0;
    let uploads_complete = lottery.uploads_complete || lottery.settlement_complete;
    if already_started || already_ended || upload_started || settlement_started || uploads_complete {
        solana_program::msg!(
            "Invalid phase transition: BeginBuyPhase lottery_id={} current_phase={} now={} buy_start={} buy_deadline={} upload_start={} settlement_start={} uploads_complete={} settlement_complete={} settled={}",
            lottery.id,
            current_phase,
            now,
            lottery.buy_start_unix,
            lottery.buy_deadline_unix,
            lottery.upload_start_unix,
            lottery.settlement_start_unix,
            lottery.uploads_complete,
            lottery.settlement_complete,
            lottery.settled
        );
        return Err(Error::InvalidPhaseTransition.into());
    }
    if lottery.buy_start_unix == 0 || now < lottery.buy_start_unix {
        lottery.buy_start_unix = now;
    }
    let bw = if buy_secs == 0 {
        config.buy_window_secs as i64
    } else {
        buy_secs as i64
    };
    let desired_deadline = now + bw;
    if desired_deadline > lottery.buy_deadline_unix {
        lottery.buy_deadline_unix = desired_deadline;
    }

    write_account_data(lottery_ai, "Lottery", &lottery)?;

    LotteryEvent::BuyPhaseBegan {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        buy_start_unix: lottery.buy_start_unix,
        buy_deadline_unix: lottery.buy_deadline_unix,
        timestamp: now,
    }
    .emit();

    Ok(())
}

