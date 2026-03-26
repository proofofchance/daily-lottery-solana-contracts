//! # Validation Utilities
//!
//! This module provides validation functions for various inputs and states
//! in the daily lottery program.

use crate::error::Error;
use solana_program::{
    account_info::AccountInfo,
    program_error::ProgramError,
    pubkey::Pubkey,
    sysvar::{clock::Clock, Sysvar},
};

/// Validates that an account is a signer
///
/// ## Parameters
/// - `account_info`: Account to validate
///
/// ## Returns
/// - `Ok(())` if account is a signer
/// - `Err(Error::Unauthorized)` if account is not a signer
pub fn require_signer(account_info: &AccountInfo) -> Result<(), ProgramError> {
    if !account_info.is_signer {
        return Err(Error::Unauthorized.into());
    }
    Ok(())
}

/// Validates that an account matches an expected pubkey
///
/// ## Parameters
/// - `account_info`: Account to validate
/// - `expected`: Expected pubkey
///
/// ## Returns
/// - `Ok(())` if keys match
/// - `Err(Error::InvalidAccountData)` if keys don't match
pub fn require_key_match(
    account_info: &AccountInfo,
    expected: &Pubkey,
) -> Result<(), ProgramError> {
    if account_info.key != expected {
        return Err(Error::InvalidAccountData.into());
    }
    Ok(())
}

/// Validates that a ticket count is valid (non-zero)
///
/// ## Parameters
/// - `count`: Number of tickets
///
/// ## Returns
/// - `Ok(())` if count is valid
/// - `Err(Error::InvalidTicketCount)` if count is zero
pub fn validate_ticket_count(count: u64) -> Result<(), ProgramError> {
    if count == 0 {
        return Err(Error::InvalidTicketCount.into());
    }
    Ok(())
}

/// Validates that a service charge rate is within acceptable bounds
///
/// Service charge must be less than 10,000 basis points (100%).
///
/// ## Parameters
/// - `bps`: Service charge in basis points
///
/// ## Returns
/// - `Ok(())` if rate is valid
/// - `Err(Error::InvalidServiceCharge)` if rate is too high
pub fn validate_service_charge(bps: u16) -> Result<(), ProgramError> {
    if bps >= 10_000 {
        return Err(Error::InvalidServiceCharge.into());
    }
    Ok(())
}

/// Computes the service fee in lamports using safe arithmetic.
pub fn compute_service_fee(total_funds: u64, bps: u16) -> Result<u64, ProgramError> {
    let fee = (u128::from(total_funds) * u128::from(bps)) / 10_000u128;
    u64::try_from(fee).map_err(|_| Error::MathOverflow.into())
}

/// Validates that current time is within a specified window
///
/// ## Parameters
/// - `start_time`: Window start time (unix timestamp)
/// - `end_time`: Window end time (unix timestamp)
///
/// ## Returns
/// - `Ok(current_time)` if within window
/// - `Err(Error::OutsideTimeWindow)` if outside window
pub fn validate_time_window(start_time: i64, end_time: i64) -> Result<i64, ProgramError> {
    let clock = Clock::get().map_err(|_| ProgramError::UnsupportedSysvar)?;
    let current_time = clock.unix_timestamp;

    if current_time < start_time || current_time > end_time {
        return Err(Error::OutsideTimeWindow.into());
    }

    Ok(current_time)
}

/// Validates that a time window is properly ordered
///
/// ## Parameters
/// - `start_time`: Window start time
/// - `end_time`: Window end time
///
/// ## Returns
/// - `Ok(())` if start < end
/// - `Err(Error::InvalidRevealWindow)` if start >= end
pub fn validate_time_window_order(start_time: i64, end_time: i64) -> Result<(), ProgramError> {
    if end_time <= start_time {
        return Err(Error::InvalidUploadWindow.into());
    }
    Ok(())
}

/// Validates that two proof-of-chance hashes match
///
/// ## Parameters
/// - `stored_hash`: Hash stored in participant account
/// - `provided_hash`: Hash provided in instruction
///
/// ## Returns
/// - `Ok(())` if hashes match
/// - `Err(Error::ProofHashMismatch)` if hashes don't match
pub fn validate_proof_hash_match(
    stored_hash: &[u8; 32],
    provided_hash: &[u8; 32],
) -> Result<(), ProgramError> {
    if stored_hash != provided_hash {
        return Err(Error::ProofHashMismatch.into());
    }
    Ok(())
}

/// Validates that an account has sufficient lamports for an operation
///
/// ## Parameters
/// - `account_info`: Account to check
/// - `required_lamports`: Minimum lamports needed
///
/// ## Returns
/// - `Ok(())` if account has sufficient lamports
/// - `Err(Error::InsufficientFunds)` if account has insufficient lamports
pub fn validate_sufficient_lamports(
    account_info: &AccountInfo,
    required_lamports: u64,
) -> Result<(), ProgramError> {
    if account_info.lamports() < required_lamports {
        return Err(Error::InsufficientFunds.into());
    }
    Ok(())
}

/// Validates that a mathematical operation won't overflow
///
/// ## Parameters
/// - `a`: First operand
/// - `b`: Second operand
///
/// ## Returns
/// - `Ok(result)` if addition is safe
/// - `Err(Error::MathOverflow)` if addition would overflow
pub fn checked_add(a: u64, b: u64) -> Result<u64, ProgramError> {
    a.checked_add(b).ok_or_else(|| Error::MathOverflow.into())
}

/// Validates that a multiplication won't overflow
///
/// ## Parameters
/// - `a`: First operand
/// - `b`: Second operand
///
/// ## Returns
/// - `Ok(result)` if multiplication is safe
/// - `Err(Error::MathOverflow)` if multiplication would overflow
pub fn checked_mul(a: u64, b: u64) -> Result<u64, ProgramError> {
    a.checked_mul(b).ok_or_else(|| Error::MathOverflow.into())
}

/// Validates that an account is writable
///
/// ## Parameters
/// - `account_info`: Account to validate
///
/// ## Returns
/// - `Ok(())` if account is writable
/// - `Err(Error::InvalidInstruction)` if account is not writable
pub fn require_writable(account_info: &AccountInfo) -> Result<(), ProgramError> {
    if !account_info.is_writable {
        return Err(Error::InvalidInstruction.into());
    }
    Ok(())
}

/// Validates that the current time is within the specified window
///
/// ## Parameters
/// - `current_time`: Current unix timestamp
/// - `start`: Window start time
/// - `end`: Window end time
///
/// ## Returns
/// - `Ok(())` if within window
/// - `Err(Error::OutsideTimeWindow)` if outside window
pub fn require_time_in_window(current_time: i64, start: i64, end: i64) -> Result<(), ProgramError> {
    if current_time < start || current_time > end {
        return Err(Error::OutsideTimeWindow.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::{account_info::AccountInfo, pubkey::Pubkey};

    fn create_test_account(key: Pubkey, is_signer: bool, lamports: u64) -> AccountInfo<'static> {
        let key_ref = Box::leak(Box::new(key));
        let owner = Box::leak(Box::new(Pubkey::new_unique()));
        let lamports_ref = Box::leak(Box::new(lamports));
        let data_vec = Vec::<u8>::new();
        let data_slice: &'static mut [u8] = Box::leak(data_vec.into_boxed_slice());

        AccountInfo::new(
            key_ref,
            is_signer,
            false,
            lamports_ref,
            data_slice,
            owner,
            false,
        )
    }

    #[test]
    fn test_require_signer() {
        let key = Pubkey::new_unique();
        let signer_account = create_test_account(key, true, 0);
        let non_signer_account = create_test_account(key, false, 0);

        assert!(require_signer(&signer_account).is_ok());
        assert!(require_signer(&non_signer_account).is_err());
    }

    #[test]
    fn test_require_key_match() {
        let key1 = Pubkey::new_unique();
        let key2 = Pubkey::new_unique();
        let account = create_test_account(key1, false, 0);

        assert!(require_key_match(&account, &key1).is_ok());
        assert!(require_key_match(&account, &key2).is_err());
    }

    #[test]
    fn test_validate_ticket_count() {
        assert!(validate_ticket_count(1).is_ok());
        assert!(validate_ticket_count(100).is_ok());
        assert!(validate_ticket_count(0).is_err());
    }

    #[test]
    fn test_validate_service_charge() {
        assert!(validate_service_charge(0).is_ok());
        assert!(validate_service_charge(500).is_ok());
        assert!(validate_service_charge(9999).is_ok());
        assert!(validate_service_charge(10000).is_err());
        assert!(validate_service_charge(15000).is_err());
    }

    #[test]
    fn test_compute_service_fee() {
        let fee = compute_service_fee(10_000, 250).unwrap();
        assert_eq!(fee, 250);

        let fee_max = compute_service_fee(u64::MAX, 9999).unwrap();
        assert!(fee_max <= u64::MAX);
    }

    #[test]
    fn test_validate_time_window_order() {
        assert!(validate_time_window_order(100, 200).is_ok());
        assert!(validate_time_window_order(200, 100).is_err());
        assert!(validate_time_window_order(100, 100).is_err());
    }

    #[test]
    fn test_validate_proof_hash_match() {
        let hash1 = [1u8; 32];
        let hash2 = [2u8; 32];

        assert!(validate_proof_hash_match(&hash1, &hash1).is_ok());
        assert!(validate_proof_hash_match(&hash1, &hash2).is_err());
    }

    #[test]
    fn test_validate_sufficient_lamports() {
        let account = create_test_account(Pubkey::new_unique(), false, 1000);

        assert!(validate_sufficient_lamports(&account, 500).is_ok());
        assert!(validate_sufficient_lamports(&account, 1000).is_ok());
        assert!(validate_sufficient_lamports(&account, 1500).is_err());
    }

    #[test]
    fn test_checked_math() {
        assert_eq!(checked_add(100, 200).unwrap(), 300);
        assert_eq!(checked_mul(10, 20).unwrap(), 200);

        assert!(checked_add(u64::MAX, 1).is_err());
        assert!(checked_mul(u64::MAX, 2).is_err());
    }
}
