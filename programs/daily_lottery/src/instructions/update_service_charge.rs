//! # Update Service Charge Instruction
//!
//! Updates the service charge rate in the global configuration.
//! Only the authority can perform this operation.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::Config,
    utils::{
        account::{read_account_data, write_account_data},
        pda::assert_pda_owned,
        validation::{require_signer, validate_service_charge},
    },
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::Sysvar,
};

/// Process the UpdateServiceCharge instruction
///
/// Updates the service charge rate in the Config account.
/// Only the authority can update the service charge.
///
/// ## Accounts Expected
/// 0. `[writable]` Config account
/// 1. `[signer]` Authority
///
/// ## Parameters
/// - `new_bps`: New service charge in basis points (0-9999)
pub fn process(program_id: &Pubkey, accounts: &[AccountInfo], new_bps: u16) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    // Get accounts
    let config_ai = next_account_info(account_info_iter)?;
    let authority_ai = next_account_info(account_info_iter)?;

    // Validate accounts
    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_signer(authority_ai)?;

    // Validate new service charge rate
    validate_service_charge(new_bps)?;

    // Read config
    let mut config: Config = read_account_data(config_ai)?;

    // Verify authority
    if authority_ai.key != &config.authority {
        return Err(Error::Unauthorized.into());
    }

    // Store old value for event
    let old_bps = config.service_charge_bps;

    // Update service charge
    config.service_charge_bps = new_bps;

    // Write updated config
    write_account_data(config_ai, "Config", &config)?;

    // Emit event
    let clock = Clock::get()?;
    let event = LotteryEvent::ServiceChargeUpdated {
        config: config_ai.key.to_string(),
        old_bps,
        new_bps,
        authority: config.authority.to_string(),
        timestamp: clock.unix_timestamp,
    };
    event.emit();

    Ok(())
}
