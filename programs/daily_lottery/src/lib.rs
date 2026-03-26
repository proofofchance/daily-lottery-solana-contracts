//! # ProofOfChance Daily Lottery Program
//!
//! A transparent, decentralized daily lottery system built on Solana that uses
//! participant-provided entropy (proof-of-chance) for fair winner selection.
//!
//! ## Core Concepts
//!
//! ### Proof-of-Chance Model
//! - Participants provide a secret phrase that gets hashed on-chain
//! - During reveal window, participants upload plaintext to service provider
//! - Provider uploads all reveals on-chain for transparent entropy generation
//! - Winner is selected deterministically using aggregated entropy
//!
//! ### Anti-Censorship Design
//! - Participants can attest on-chain that they've uploaded their reveal
//! - Provider must include all attested participants in reveal batch
//! - If any attested participant is missing, settlement is blocked
//!
//! ### Single Active Lottery Constraint
//! - Only one lottery can be active at a time
//! - New lottery can only be created after current one is settled
//! - Ensures focused participation and clear lifecycle management
//!
//! ## Account Architecture
//!
//! All accounts use Program Derived Addresses (PDAs) for security:
//! - **Config**: `["config"]` - Global system configuration
//! - **Lottery**: `["lottery", config_pubkey, lottery_id_le_bytes]` - Individual lottery state
//! - **Participant**: `["participant", lottery_pubkey, wallet_pubkey]` - Participant data
//! - **Vault**: `["vault", lottery_pubkey]` - Fund custody account
//!
//! ## Instruction Flow
//!
//! 1. **Initialize**: Set up global configuration (once per deployment)
//! 2. **CreateLottery**: Authority creates new lottery instance
//! 3. **BuyTickets**: Participants purchase tickets with proof-of-chance hash
//! 4. **AttestUploaded**: Participants attest to off-chain reveal upload
//! 5. **UploadReveals**: Provider uploads batch of reveals for settlement
//! 6. **Settle**: Determine winner and distribute funds
//!
//! ## Security Features
//!
//! - **PDA-based accounts**: All accounts use program-derived addresses
//! - **Authority checks**: Critical operations require proper authorization
//! - **Time window enforcement**: Strict timing for reveal windows
//! - **Input validation**: Comprehensive validation of all parameters
//! - **Overflow protection**: Safe arithmetic throughout

#![deny(unsafe_code)]
#![allow(unexpected_cfgs)]

// Re-export key types for external use
pub use error::Error;
pub use instructions::Instruction;
pub use state::{Config, Lottery, Participant, Vault};

// Internal modules
pub mod error;
pub mod events;
pub mod instructions;
pub mod state;
pub mod utils;

// Solana program imports
use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult, pubkey::Pubkey,
};

// Program entry point
entrypoint!(process_instruction);

/// Main program entry point
///
/// This function is called by the Solana runtime for every transaction
/// that invokes this program. It deserializes the instruction data and
/// routes it to the appropriate instruction handler.
///
/// ## Parameters
/// - `program_id`: The public key of this program
/// - `accounts`: Array of accounts involved in the transaction
/// - `instruction_data`: Serialized instruction data
///
/// ## Returns
/// - `Ok(())` if instruction executed successfully
/// - `Err(ProgramError)` if any error occurred during execution
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    instructions::process_instruction(program_id, accounts, instruction_data)
}
