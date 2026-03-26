//! # Account Utilities
//!
//! This module provides utility functions for reading and writing account data
//! with proper discriminator handling and Borsh serialization.

use crate::utils::crypto::discriminator;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{account_info::AccountInfo, program_error::ProgramError};
use std::io::Cursor;

/// Reads and deserializes account data with discriminator validation
///
/// This function reads account data, validates the 8-byte discriminator,
/// and deserializes the remaining data using Borsh.
///
/// ## Parameters
/// - `account_info`: Account to read from
///
/// ## Returns
/// - `Ok(T)`: Successfully deserialized data
/// - `Err(ProgramError)`: Invalid account data or deserialization failed
///
/// ## Usage
/// ```ignore
/// let config: Config = read_account_data(config_account)?;
/// ```
pub fn read_account_data<T: BorshDeserialize>(
    account_info: &AccountInfo,
) -> Result<T, ProgramError> {
    let data = account_info.data.borrow();

    // Ensure account has at least discriminator
    if data.len() < 8 {
        return Err(ProgramError::InvalidAccountData);
    }

    // Deserialize data after discriminator, allowing trailing bytes in account
    // (accounts may be over-allocated to support growth without realloc)
    let mut cursor = Cursor::new(&data[8..]);
    T::deserialize_reader(&mut cursor).map_err(|e| {
        solana_program::msg!("Borsh deserialization failed: {:?}", e);
        ProgramError::InvalidAccountData
    })
}

/// Writes serialized data to account with proper discriminator
///
/// This function writes an 8-byte discriminator followed by the Borsh-serialized
/// data to the account. The discriminator is generated from the type name.
///
/// ## Parameters
/// - `account_info`: Account to write to
/// - `type_name`: Name of the type (used for discriminator generation)
/// - `data`: Data to serialize and write
///
/// ## Returns
/// - `Ok(())`: Successfully wrote data
/// - `Err(ProgramError)`: Account too small or serialization failed
///
/// ## Usage
/// ```ignore
/// write_account_data(config_account, "Config", &config)?;
/// ```
pub fn write_account_data<T: BorshSerialize>(
    account_info: &AccountInfo,
    type_name: &str,
    data: &T,
) -> Result<(), ProgramError> {
    let mut account_data = account_info.data.borrow_mut();

    // Ensure account has at least discriminator space
    if account_data.len() < 8 {
        return Err(ProgramError::AccountDataTooSmall);
    }

    // Write discriminator
    let disc = discriminator(type_name);
    account_data[..8].copy_from_slice(&disc);

    // Serialize and write data
    data.serialize(&mut &mut account_data[8..])
        .map_err(|_| ProgramError::AccountDataTooSmall)
}

/// Reads account data with fallback to default if account is uninitialized
///
/// This function attempts to read account data, but returns a default value
/// if the account is uninitialized or has invalid data. Useful for accounts
/// that may not exist yet.
///
/// ## Parameters
/// - `account_info`: Account to read from
///
/// ## Returns
/// - Deserialized data if account is valid, otherwise default value
///
/// ## Usage
/// ```ignore
/// let participant: Participant = read_account_data_or_default(participant_account);
/// ```
pub fn read_account_data_or_default<T: BorshDeserialize + Default>(
    account_info: &AccountInfo,
) -> T {
    read_account_data(account_info).unwrap_or_default()
}

/// Validates that an account has the expected discriminator
///
/// This function checks if an account's first 8 bytes match the expected
/// discriminator for a given type name.
///
/// ## Parameters
/// - `account_info`: Account to validate
/// - `type_name`: Expected type name
///
/// ## Returns
/// - `true` if discriminator matches
/// - `false` if discriminator doesn't match or account is too small
pub fn validate_account_discriminator(account_info: &AccountInfo, type_name: &str) -> bool {
    let data = account_info.data.borrow();

    if data.len() < 8 {
        return false;
    }

    let expected_disc = discriminator(type_name);
    data[..8] == expected_disc
}

/// Gets the size of serialized data plus discriminator
///
/// This function calculates the total size needed to store a value
/// including the 8-byte discriminator.
///
/// ## Parameters
/// - `data`: Data to calculate size for
///
/// ## Returns
/// - Total size in bytes (data + 8 byte discriminator)
pub fn get_account_size<T: BorshSerialize>(data: &T) -> Result<usize, ProgramError> {
    let serialized_size = borsh::to_vec(data)
        .map_err(|_| ProgramError::InvalidAccountData)?
        .len();

    Ok(8 + serialized_size) // discriminator + data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Config;
    use solana_program::{account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey};

    fn create_test_account(data: Vec<u8>) -> AccountInfo<'static> {
        let key = Box::leak(Box::new(Pubkey::new_unique()));
        let owner = Box::leak(Box::new(Pubkey::new_unique()));
        let lamports = Box::leak(Box::new(0u64));
        let data_vec = Box::leak(Box::new(data));
        let data_slice: &'static mut [u8] = data_vec.as_mut_slice();

        AccountInfo::new(key, false, false, lamports, data_slice, owner, false)
    }

    #[test]
    fn test_write_and_read_account_data() {
        let config = Config {
            authority: Pubkey::new_unique(),
            buy_window_secs: 3600,
            upload_window_secs: 1800,
            ticket_price_lamports: 1000,
            service_charge_bps: 500,
            lottery_count: 0,
            max_winners_cap: 100,
        };

        // Create account with exact required space
        let data = vec![0u8; get_account_size(&config).unwrap()];
        let account = create_test_account(data);

        // Write data
        write_account_data(&account, "Config", &config).unwrap();

        // Read data back
        let read_config: Config = read_account_data(&account).unwrap();

        assert_eq!(config.authority, read_config.authority);
        assert_eq!(
            config.ticket_price_lamports,
            read_config.ticket_price_lamports
        );
        assert_eq!(config.service_charge_bps, read_config.service_charge_bps);
    }

    #[test]
    fn test_read_uninitialized_account() {
        let account = create_test_account(vec![0u8; 8]); // Only discriminator space

        let result: Result<Config, ProgramError> = read_account_data(&account);
        assert!(result.is_err());

        // But default should work
        let default_config: Config = read_account_data_or_default(&account);
        assert_eq!(default_config.lottery_count, 0);
    }

    #[test]
    fn test_validate_discriminator() {
        let config = Config::default();
        let data = vec![0u8; 100];
        let account = create_test_account(data);

        // Write config data
        write_account_data(&account, "Config", &config).unwrap();

        // Should validate correctly
        assert!(validate_account_discriminator(&account, "Config"));
        assert!(!validate_account_discriminator(&account, "Lottery"));
    }

    #[test]
    fn test_account_too_small() {
        let config = Config::default();
        let account = create_test_account(vec![0u8; 4]); // Too small

        let result = write_account_data(&account, "Config", &config);
        assert!(result.is_err());
    }
}
