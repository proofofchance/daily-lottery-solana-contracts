//! # Error Types for Daily Lottery Program
//!
//! This module defines all custom error types used throughout the daily lottery program.
//! Each error has a specific meaning and helps with debugging and user feedback.

use solana_program::program_error::ProgramError;
use thiserror::Error;

/// Custom error types for the daily lottery program
///
/// These errors provide specific context about what went wrong during
/// program execution, making debugging and user feedback more effective.
#[derive(Error, Debug, Copy, Clone, PartialEq, Eq)]
pub enum Error {
    /// Invalid instruction data was provided
    #[error("Invalid instruction data")]
    InvalidInstruction,

    /// A mathematical operation would overflow
    #[error("Mathematical operation overflow")]
    MathOverflow,

    /// Insufficient funds for the requested operation
    #[error("Insufficient funds")]
    InsufficientFunds,

    /// The operation is not authorized for this account
    #[error("Unauthorized operation")]
    Unauthorized,

    /// The lottery is not in the correct state for this operation
    #[error("Invalid lottery state")]
    InvalidLotteryState,

    /// The upload window timing is invalid
    #[error("Invalid upload window")]
    InvalidUploadWindow,

    /// The attestation signature is invalid or missing
    #[error("Invalid attestation")]
    InvalidAttestation,

    /// The provided uploads don't match the expected participants
    #[error("Invalid uploads")]
    InvalidUploads,

    /// The lottery has already been settled
    #[error("Lottery already settled")]
    AlreadySettled,

    /// No active lottery exists when one is required
    #[error("No active lottery")]
    NoActiveLottery,

    /// An active lottery already exists when creating a new one
    #[error("Active lottery exists")]
    ActiveLotteryExists,

    /// The participant has already attested for this lottery
    #[error("Already attested")]
    AlreadyAttested,

    /// The proof of chance hash doesn't match the expected value
    #[error("Proof hash mismatch")]
    ProofHashMismatch,

    /// The operation is outside the allowed time window
    #[error("Outside time window")]
    OutsideTimeWindow,

    /// Invalid account data or discriminator
    #[error("Invalid account data")]
    InvalidAccountData,

    /// Required account is missing from instruction
    #[error("Missing required account")]
    MissingAccount,

    /// Account has incorrect owner
    #[error("Incorrect account owner")]
    IncorrectOwner,

    /// PDA seeds don't match expected values
    #[error("Invalid PDA seeds")]
    InvalidSeeds,

    /// Service charge rate is out of valid range
    #[error("Invalid service charge")]
    InvalidServiceCharge,

    /// Ticket count is invalid (zero or too large)
    #[error("Invalid ticket count")]
    InvalidTicketCount,

    /// Winner determination failed
    #[error("Winner selection failed")]
    WinnerSelectionFailed,

    /// Lottery has already been settled
    #[error("Lottery already settled")]
    LotteryAlreadySettled,

    /// Winner could not be found
    #[error("Winner not found")]
    WinnerNotFound,

    /// Reveal data doesn't match proof-of-chance hash
    #[error("Reveal mismatch")]
    RevealMismatch,

    /// Missing proof-of-chance data
    #[error("Missing proof of chance")]
    MissingProofOfChance,

    /// Lottery is not settled yet
    #[error("Lottery not settled")]
    LotteryNotSettled,

    /// Active lottery already exists
    #[error("Lottery already active")]
    LotteryAlreadyActive,

    /// Attestation already submitted
    #[error("Attestation already submitted")]
    AttestationAlreadySubmitted,

    /// Invalid time window for operation
    #[error("Invalid time window")]
    InvalidTimeWindow,

    /// No attested participants to upload reveals for
    #[error("No attested participants")]
    NoAttestedParticipants,

    /// Winner has already been paid
    #[error("Winner already paid")]
    WinnerAlreadyPaid,

    /// Invalid merkle proof provided
    #[error("Invalid merkle proof")]
    InvalidMerkleProof,

    /// Invalid account provided
    #[error("Invalid account")]
    InvalidAccount,

    /// Invalid phase transition
    #[error("Invalid phase transition")]
    InvalidPhaseTransition,
}

impl From<Error> for ProgramError {
    fn from(e: Error) -> Self {
        ProgramError::Custom(e as u32)
    }
}

impl From<Error> for u32 {
    fn from(e: Error) -> Self {
        e as u32
    }
}

/// Result type alias for program operations
pub type ProgramResult<T = ()> = Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_conversion() {
        let error = Error::InvalidInstruction;
        let program_error: ProgramError = error.into();

        match program_error {
            ProgramError::Custom(code) => assert_eq!(code, Error::InvalidInstruction as u32),
            _ => panic!("Expected custom error"),
        }
    }

    #[test]
    fn test_error_codes_unique() {
        // Ensure all error codes are unique
        let errors = [
            Error::InvalidInstruction,
            Error::MathOverflow,
            Error::InsufficientFunds,
            Error::Unauthorized,
            Error::InvalidLotteryState,
            Error::InvalidUploadWindow,
            Error::InvalidAttestation,
            Error::InvalidUploads,
            Error::AlreadySettled,
            Error::NoActiveLottery,
            Error::ActiveLotteryExists,
            Error::AlreadyAttested,
            Error::ProofHashMismatch,
            Error::OutsideTimeWindow,
            Error::InvalidAccountData,
            Error::MissingAccount,
            Error::IncorrectOwner,
            Error::InvalidSeeds,
            Error::InvalidServiceCharge,
            Error::InvalidTicketCount,
            Error::WinnerSelectionFailed,
            Error::LotteryAlreadySettled,
            Error::WinnerNotFound,
            Error::RevealMismatch,
            Error::MissingProofOfChance,
            Error::LotteryNotSettled,
            Error::LotteryAlreadyActive,
            Error::AttestationAlreadySubmitted,
            Error::InvalidTimeWindow,
            Error::NoAttestedParticipants,
            Error::WinnerAlreadyPaid,
            Error::InvalidMerkleProof,
            Error::InvalidAccount,
            Error::InvalidPhaseTransition,
        ];

        let mut codes: Vec<u32> = errors.iter().map(|e| *e as u32).collect();
        codes.sort();
        codes.dedup();

        assert_eq!(codes.len(), errors.len(), "Duplicate error codes found");
    }
}
