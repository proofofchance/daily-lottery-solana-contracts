//! # Program Derived Address (PDA) Utilities
//!
//! This module provides utilities for working with PDAs in the daily lottery program.
//! All accounts use PDAs for security and deterministic addressing.

use crate::error::Error;
use solana_program::{account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey};

/// Validates that an account is a PDA with expected seeds and is owned by the program
///
/// This function verifies that:
/// 1. The account key matches the expected PDA for the given seeds
/// 2. The account is owned by the program
///
/// ## Parameters
/// - `program_id`: The program that should own the account
/// - `account_info`: Account to validate
/// - `seeds`: Seeds used to derive the PDA
///
/// ## Returns
/// - `Ok(bump)`: PDA bump seed if validation succeeds
/// - `Err(ProgramError)`: If validation fails
pub fn assert_pda_owned(
    program_id: &Pubkey,
    account_info: &AccountInfo,
    seeds: &[&[u8]],
) -> Result<u8, ProgramError> {
    let (expected_key, bump) = Pubkey::find_program_address(seeds, program_id);
    solana_program::msg!(
        "assert_pda_owned: expected {} actual {} seeds_count {}",
        expected_key,
        account_info.key,
        seeds.len()
    );

    // Verify the account key matches expected PDA
    if expected_key != *account_info.key {
        return Err(Error::InvalidSeeds.into());
    }

    // Verify the account is owned by the program
    if account_info.owner != program_id {
        return Err(Error::IncorrectOwner.into());
    }

    Ok(bump)
}

/// Validates that an account key matches the expected PDA (without ownership check)
///
/// This function only verifies that the account key matches the expected PDA
/// for the given seeds. It doesn't check ownership, which is useful for
/// accounts that haven't been created yet.
///
/// ## Parameters
/// - `program_id`: The program used for PDA derivation
/// - `account_info`: Account to validate
/// - `seeds`: Seeds used to derive the PDA
///
/// ## Returns
/// - `Ok(bump)`: PDA bump seed if key matches
/// - `Err(ProgramError)`: If key doesn't match expected PDA
pub fn assert_pda_key(
    program_id: &Pubkey,
    account_info: &AccountInfo,
    seeds: &[&[u8]],
) -> Result<u8, ProgramError> {
    let (expected_key, bump) = Pubkey::find_program_address(seeds, program_id);

    if expected_key != *account_info.key {
        return Err(Error::InvalidSeeds.into());
    }

    Ok(bump)
}

/// Derives a PDA for the Config account
///
/// The Config account uses seeds: `["config"]`
///
/// ## Parameters
/// - `program_id`: The lottery program ID
///
/// ## Returns
/// - `(Pubkey, u8)`: The PDA address and bump seed
pub fn derive_config_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"config"], program_id)
}

/// Derives a PDA for a Lottery account
///
/// Lottery accounts use seeds: `["lottery", config_pubkey, lottery_id_le_bytes]`
///
/// ## Parameters
/// - `program_id`: The lottery program ID
/// - `config_pubkey`: The Config account address
/// - `lottery_id`: The lottery ID (will be converted to little-endian bytes)
///
/// ## Returns
/// - `(Pubkey, u8)`: The PDA address and bump seed
pub fn derive_lottery_pda(
    program_id: &Pubkey,
    config_pubkey: &Pubkey,
    lottery_id: u64,
) -> (Pubkey, u8) {
    let id_bytes = lottery_id.to_le_bytes();
    Pubkey::find_program_address(&[b"lottery", config_pubkey.as_ref(), &id_bytes], program_id)
}

/// Derives a PDA for a Participant account
///
/// Participant accounts use seeds: `["participant", lottery_pubkey, wallet_pubkey]`
///
/// ## Parameters
/// - `program_id`: The lottery program ID
/// - `lottery_pubkey`: The Lottery account address
/// - `wallet_pubkey`: The participant's wallet address
///
/// ## Returns
/// - `(Pubkey, u8)`: The PDA address and bump seed
pub fn derive_participant_pda(
    program_id: &Pubkey,
    lottery_pubkey: &Pubkey,
    wallet_pubkey: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"participant",
            lottery_pubkey.as_ref(),
            wallet_pubkey.as_ref(),
        ],
        program_id,
    )
}

/// Derives a PDA for a Vault account
///
/// Vault accounts use seeds: `["vault", lottery_pubkey]`
///
/// ## Parameters
/// - `program_id`: The lottery program ID
/// - `lottery_pubkey`: The Lottery account address
///
/// ## Returns
/// - `(Pubkey, u8)`: The PDA address and bump seed
pub fn derive_vault_pda(program_id: &Pubkey, lottery_pubkey: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault", lottery_pubkey.as_ref()], program_id)
}

/// Derives a PDA for WinnersLedger account
/// Seeds: ["winners_ledger", lottery_pubkey]
pub fn derive_winners_ledger_pda(program_id: &Pubkey, lottery_pubkey: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"winners_ledger", lottery_pubkey.as_ref()], program_id)
}

/// Derives a PDA for VoteTally account
/// Seeds: ["vote_tally", lottery_pubkey]
pub fn derive_vote_tally_pda(program_id: &Pubkey, lottery_pubkey: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vote_tally", lottery_pubkey.as_ref()], program_id)
}

/// Creates signing seeds for a PDA with bump
///
/// This helper function creates the seeds array needed for signing
/// with a PDA, including the bump seed. The bump is stored in a static
/// location to ensure proper lifetime.
///
/// ## Parameters
/// - `seeds`: Base seeds (without bump)
/// - `bump`: PDA bump seed
///
/// ## Returns
/// - Seeds array suitable for `invoke_signed`
///
/// ## Usage
/// ```ignore
/// let seeds = create_signing_seeds(&[b"config"], bump);
/// invoke_signed(&instruction, accounts, &[&seeds])?;
/// ```
pub fn create_signing_seeds(seeds: &[&[u8]], bump: u8) -> Vec<Vec<u8>> {
    let mut signing_seeds: Vec<Vec<u8>> = seeds.iter().map(|s| s.to_vec()).collect();
    signing_seeds.push(vec![bump]);
    signing_seeds
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;

    #[test]
    fn test_derive_config_pda() {
        let program_id = Pubkey::new_unique();
        let (pda, bump) = derive_config_pda(&program_id);

        // Verify we can reproduce the same PDA
        let (expected, expected_bump) = Pubkey::find_program_address(&[b"config"], &program_id);
        assert_eq!(pda, expected);
        assert_eq!(bump, expected_bump);
    }

    #[test]
    fn test_derive_lottery_pda() {
        let program_id = Pubkey::new_unique();
        let config = Pubkey::new_unique();
        let lottery_id = 42u64;

        let (pda, bump) = derive_lottery_pda(&program_id, &config, lottery_id);

        // Verify we can reproduce the same PDA
        let id_bytes = lottery_id.to_le_bytes();
        let (expected, expected_bump) =
            Pubkey::find_program_address(&[b"lottery", config.as_ref(), &id_bytes], &program_id);
        assert_eq!(pda, expected);
        assert_eq!(bump, expected_bump);
    }

    #[test]
    fn test_derive_participant_pda() {
        let program_id = Pubkey::new_unique();
        let lottery = Pubkey::new_unique();
        let wallet = Pubkey::new_unique();

        let (pda, bump) = derive_participant_pda(&program_id, &lottery, &wallet);

        // Verify we can reproduce the same PDA
        let (expected, expected_bump) = Pubkey::find_program_address(
            &[b"participant", lottery.as_ref(), wallet.as_ref()],
            &program_id,
        );
        assert_eq!(pda, expected);
        assert_eq!(bump, expected_bump);
    }

    #[test]
    fn test_derive_vault_pda() {
        let program_id = Pubkey::new_unique();
        let lottery = Pubkey::new_unique();

        let (pda, bump) = derive_vault_pda(&program_id, &lottery);

        // Verify we can reproduce the same PDA
        let (expected, expected_bump) =
            Pubkey::find_program_address(&[b"vault", lottery.as_ref()], &program_id);
        assert_eq!(pda, expected);
        assert_eq!(bump, expected_bump);
    }

    #[test]
    fn test_create_signing_seeds() {
        let bump = 255u8;
        let seeds = create_signing_seeds(&[b"config"], bump);

        assert_eq!(seeds.len(), 2);
        assert_eq!(seeds[0], b"config".to_vec());
        assert_eq!(seeds[1], vec![bump]);
    }
}
