//! # Initialize Instruction
//!
//! Creates the global Config account and sets up the lottery system.
//! This instruction can only be called once per program deployment.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{sizes::CONFIG_SIZE, Config},
    utils::account::*,
    utils::validation::require_key_match,
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    program::invoke_signed,
    pubkey::Pubkey,
    sysvar::rent::Rent,
    sysvar::Sysvar,
};
use solana_system_interface::{instruction as system_instruction, program as system_program};

/// Process the Initialize instruction
///
/// Creates the global Config account with initial lottery system parameters.
/// The Config account uses PDA seeds `["config"]` and stores system-wide settings.
///
/// ## Accounts Expected
/// 0. `[writable, signer]` Authority (pays for account creation)
/// 1. `[writable]` Config account (PDA with seeds `["config"]`)
/// 2. `[]` System program
///
/// ## Parameters
/// - `ticket_price_lamports`: Price per lottery ticket in lamports (immutable)
/// - `service_charge_bps`: Service charge in basis points 0-9999 (updatable)
/// - `max_winners_cap`: Upper bound for winners bitmap sizing at lottery creation
pub fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    ticket_price_lamports: u64,
    service_charge_bps: u16,
    max_winners_cap: u32,
) -> ProgramResult {
    // Parse accounts
    let account_iter = &mut accounts.iter();
    let authority_ai = next_account_info(account_iter)?;
    let config_ai = next_account_info(account_iter)?;
    let system_program_ai = next_account_info(account_iter)?;

    // Validate authority signature
    if !authority_ai.is_signer {
        return Err(Error::Unauthorized.into());
    }
    require_key_match(system_program_ai, &system_program::id())?;

    // Validate service charge rate
    if !Config::validate_service_charge(service_charge_bps) {
        return Err(Error::InvalidServiceCharge.into());
    }

    // Verify config PDA
    let config_seeds: &[&[u8]] = &[b"config"];
    let (expected_config, bump) = Pubkey::find_program_address(config_seeds, program_id);
    if expected_config != *config_ai.key {
        return Err(Error::InvalidSeeds.into());
    }

    // Create config account if it doesn't exist; otherwise return early (idempotent)
    if config_ai.owner != program_id {
        let rent = Rent::get()?;
        solana_program::msg!("CONFIG_SIZE: {}", CONFIG_SIZE);
        let lamports = rent.minimum_balance(CONFIG_SIZE);

        let create_ix = system_instruction::create_account(
            authority_ai.key,
            config_ai.key,
            lamports,
            CONFIG_SIZE as u64,
            program_id,
        );

        invoke_signed(
            &create_ix,
            &[
                authority_ai.clone(),
                config_ai.clone(),
                system_program_ai.clone(),
            ],
            &[&[b"config", &[bump]]],
        )?;

        // Initialize config data for newly created account
        let config = Config {
            authority: *authority_ai.key,
            ticket_price_lamports,
            service_charge_bps,
            lottery_count: 0,
            buy_window_secs: 24 * 60 * 60,
            upload_window_secs: 24 * 60 * 60,
            max_winners_cap,
        };

        write_account_data(config_ai, "Config", &config)?;

        // Emit event
        let clock = Clock::get()?;
        let event = LotteryEvent::SystemInitialized {
            authority: authority_ai.key.to_string(),
            config: config_ai.key.to_string(),
            ticket_price_lamports,
            service_charge_bps,
            timestamp: clock.unix_timestamp,
        };
        event.emit();

        return Ok(());
    }

    // Config already exists and is owned by this program; do not overwrite
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_parameters() {
        // Valid service charges
        assert!(Config::validate_service_charge(0));
        assert!(Config::validate_service_charge(500)); // 5%
        assert!(Config::validate_service_charge(9999)); // 99.99%

        // Invalid service charges
        assert!(!Config::validate_service_charge(10000)); // 100%
        assert!(!Config::validate_service_charge(15000)); // 150%
    }
}
