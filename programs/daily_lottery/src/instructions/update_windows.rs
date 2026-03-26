//! # Update Windows Instruction (Admin)
//!
//! Authority-only. Updates `Config.buy_window_secs` and `Config.upload_window_secs`.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::Config,
    utils::{account::read_account_data, account::write_account_data, pda::assert_pda_owned, validation::require_signer, validation::require_writable},
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
    buy_secs: u32,
    upload_secs: u32,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let config_ai = next_account_info(account_info_iter)?;
    let authority_ai = next_account_info(account_info_iter)?;

    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(config_ai)?;
    require_signer(authority_ai)?;

    let mut config: Config = read_account_data(config_ai)?;
    if authority_ai.key != &config.authority {
        return Err(Error::Unauthorized.into());
    }

    // Store old values for event
    let old_buy_window_secs = config.buy_window_secs;
    let old_upload_window_secs = config.upload_window_secs;

    if buy_secs > 0 {
        config.buy_window_secs = buy_secs;
    }
    if upload_secs > 0 {
        config.upload_window_secs = upload_secs;
    }

    write_account_data(config_ai, "Config", &config)?;

    // Emit event
    let clock = Clock::get()?;
    let event = LotteryEvent::WindowsUpdated {
        config: config_ai.key.to_string(),
        old_buy_window_secs,
        new_buy_window_secs: config.buy_window_secs,
        old_upload_window_secs,
        new_upload_window_secs: config.upload_window_secs,
        authority: config.authority.to_string(),
        timestamp: clock.unix_timestamp,
    };
    event.emit();

    Ok(())
}


