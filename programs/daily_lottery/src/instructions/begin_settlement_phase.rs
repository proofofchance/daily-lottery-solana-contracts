//! # Begin Settlement Phase Instruction
//!
//! Authority-only (or anyone, if you prefer) to mark settlement as begun.
//! Sets `settlement_start_unix = now` if not set and conditions met.

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

pub fn process(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let authority_ai = next_account_info(account_info_iter)?;

    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(lottery_ai)?;
    require_signer(authority_ai)?;

    let config: Config = read_account_data(config_ai)?;
    let mut lottery: Lottery = read_account_data(lottery_ai)?;

    let (expected_lottery, _bump) = derive_lottery_pda(program_id, &lottery.config, lottery.id);
    require_key_match(lottery_ai, &expected_lottery)?;

    // Authority check
    if authority_ai.key != &config.authority {
        return Err(Error::Unauthorized.into());
    }

    if lottery.settled {
        return Err(Error::LotteryAlreadySettled.into());
    }

    let now = Clock::get()?.unix_timestamp;
    let current_phase = lottery.phase(now);
    let settlement_started = lottery.settlement_start_unix > 0;
    if settlement_started || lottery.settlement_complete {
        solana_program::msg!(
            "Invalid phase transition: BeginSettlementPhase lottery_id={} current_phase={} now={} settlement_start={} settlement_complete={} settled={}",
            lottery.id,
            current_phase,
            now,
            lottery.settlement_start_unix,
            lottery.settlement_complete,
            lottery.settled
        );
        return Err(Error::InvalidPhaseTransition.into());
    }

    // Conditions: upload window elapsed OR everyone attested
    let upload_elapsed = lottery.upload_deadline_unix > 0 && now > lottery.upload_deadline_unix;
    let all_attested = lottery.participants_count > 0 && lottery.attested_count == lottery.participants_count;
    if !(upload_elapsed || all_attested) {
        return Err(Error::InvalidInstruction.into());
    }

    if lottery.settlement_start_unix == 0 {
        lottery.settlement_start_unix = now;
        write_account_data(lottery_ai, "Lottery", &lottery)?;
        LotteryEvent::SettlementPhaseBegan {
            lottery_id: lottery.id,
            lottery: lottery_ai.key.to_string(),
            settlement_start_unix: now,
            timestamp: now,
        }
        .emit();
    }

    Ok(())
}

