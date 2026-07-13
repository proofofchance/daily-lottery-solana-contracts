//! # Instructions Module
//!
//! This module contains all the instruction definitions and their processing logic
//! for the daily lottery program. Each instruction is implemented as a separate
//! module for better organization and RAG tool understanding.
//!
//! ## Instruction Categories
//!
//! ### Administrative Instructions
//! - [`initialize`]: Initialize the lottery system configuration
//! - [`update_service_charge`]: Update the service charge rate in feature-enabled builds
//! - [`begin_reveal_phase`]: Feature-gated local/staging upload timing control
//!
//! ### Lottery Lifecycle Instructions  
//! - [`create_lottery`]: Create a new daily lottery instance
//! - [`finalize_winners`]: Finalize winners into fixed winner-page PDAs
//! - [`settle_payout_batch`]: Process batch of winner payouts
//!
//! ### Participant Instructions
//! - [`buy_tickets`]: Purchase lottery tickets with proof-of-chance
//! - [`attest_uploaded`]: Attest to off-chain reveal upload
//! - [`attest_reveal`]: Attest and include a reveal on-chain
//! - [`claim_refund`]: Claim a program-vault refund after cancellation
//! - [`close_refund_vault`]: Reclaim refund-path vault rent after all claims
//! - [`close_participant`]: Reclaim terminal participant account rent
//!
//! ### Provider Instructions
//! - [`upload_reveals`]: Upload batch of participant reveals

use borsh::{BorshDeserialize, BorshSerialize};

pub mod attest_reveal;
pub mod attest_uploaded;
pub mod begin_reveal_phase;
pub mod buy_tickets;
pub mod claim_refund;
pub mod close_participant;
pub mod close_refund_vault;
pub mod create_lottery;
pub mod finalize_no_attesters;
pub mod finalize_winners;
pub mod initialize;
pub mod settle_payout_batch;
pub mod update_service_charge;
pub mod upload_reveals;

// Re-export specific functions to avoid ambiguous glob imports
pub use attest_reveal::process as process_attest_reveal;
pub use attest_uploaded::process as process_attest_uploaded;
pub use begin_reveal_phase::process as process_begin_reveal_phase;
pub use buy_tickets::process as process_buy_tickets;
pub use claim_refund::process as process_claim_refund;
pub use close_participant::process as process_close_participant;
pub use close_refund_vault::process as process_close_refund_vault;
pub use create_lottery::process as process_create_lottery;
pub use finalize_no_attesters::process as process_finalize_no_attesters;
pub use finalize_winners::process as process_finalize_winners;
pub use initialize::process as process_initialize;
pub use settle_payout_batch::process as process_settle_payout_batch;
pub use update_service_charge::process as process_update_service_charge;
pub use upload_reveals::process as process_upload_reveals;

/// Instruction tag constants - PINNED to prevent enum drift
/// These MUST match the frontend enumTag values exactly
pub const TAG_INITIALIZE: u8 = 0;
pub const TAG_UPDATE_SERVICE_CHARGE: u8 = 1;
pub const TAG_CREATE_LOTTERY: u8 = 2;
pub const TAG_BUY_TICKETS: u8 = 3;
pub const TAG_ATTEST_UPLOADED: u8 = 4;
pub const TAG_UPLOAD_REVEALS: u8 = 5;
pub const TAG_BEGIN_REVEAL_NOW: u8 = 6;
pub const TAG_FINALIZE_WINNERS: u8 = 7;
pub const TAG_BEGIN_REVEAL_PHASE: u8 = 8;
pub const TAG_FINALIZE_NO_ATTESTERS: u8 = 9;
pub const TAG_SETTLE_PAYOUT_BATCH: u8 = 10;
pub const TAG_CLAIM_REFUND: u8 = 14;
pub const TAG_ATTEST_REVEAL: u8 = 15;
pub const TAG_CLOSE_REFUND_VAULT: u8 = 16;
pub const TAG_CLOSE_PARTICIPANT: u8 = 17;

/// All possible instructions for the daily lottery program
///
/// Each instruction variant contains the specific parameters needed
/// for that operation. Instructions are serialized using Borsh for
/// efficient on-chain processing.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub enum Instruction {
    /// Initialize the lottery system with configuration
    ///
    /// Creates the global Config account and sets initial parameters.
    /// Can only be called once per program deployment.
    ///
    /// Accounts expected:
    /// 0. `[writable, signer]` Authority (pays for account creation)
    /// 1. `[writable]` Config account (PDA)
    /// 2. `[]` System program
    Initialize {
        /// Price per lottery ticket in lamports
        ticket_price_lamports: u64,
        /// Service charge in basis points (0-9999)
        service_charge_bps: u16,
        /// Maximum winners cap used to size settlement bitmaps
        max_winners_cap: u32,
    },

    /// Update the service charge rate
    ///
    /// Only the authority can update the service charge rate in builds with
    /// `allow-service-charge-update`.
    ///
    /// Accounts expected:
    /// 0. `[writable]` Config account
    /// 1. `[signer]` Authority
    UpdateServiceCharge {
        /// New service charge in basis points (0-9999)
        new_bps: u16,
    },

    /// Create a new daily lottery
    ///
    /// Creates a new lottery instance with its associated vault.
    /// Only one lottery can be active at a time.
    ///
    /// Accounts expected:
    /// 0. `[writable]` Config account
    /// 1. `[writable]` Lottery account (PDA)
    /// 2. `[writable]` Vault account (PDA)
    /// 3. `[signer]` Authority
    /// 4. `[]` System program
    CreateLottery,

    /// Purchase tickets for the current lottery
    ///
    /// Participants can buy multiple tickets and must provide their
    /// proof-of-chance hash on first purchase.
    ///
    /// Accounts expected:
    /// 0. `[]` Config account
    /// 1. `[writable]` Lottery account
    /// 2. `[writable]` Vault account
    /// 3. `[writable]` Participant account (PDA)
    /// 4. `[writable, signer]` Payer (participant wallet)
    /// 5. `[]` System program
    BuyTickets {
        /// Proof-of-chance hash (required on first purchase, optional on subsequent)
        proof_of_chance_hash: Option<[u8; 32]>,
        /// Number of tickets to purchase
        number_of_tickets: u64,
    },

    /// Attest to having uploaded reveal off-chain
    ///
    /// Participants use this to prove they've uploaded their reveal
    /// to the service provider, preventing censorship.
    ///
    /// Accounts expected:
    /// 0. `[]` Config account
    /// 1. `[writable]` Lottery account
    /// 2. `[writable]` Participant account
    /// 3. `[signer]` Participant wallet
    /// 4. `[]` Instructions sysvar
    AttestUploaded {
        /// Participant's vote for number of winners
        voted_number_of_winners: u64,
    },

    /// Upload batch of reveals for settlement (V2 explicit mapping)
    ///
    /// Service provider uploads explicit (participant, plaintext) pairs
    /// for attested participants. Order is not trusted; program re-sorts
    /// deterministically by participant pubkey when aggregating entropy.
    ///
    /// Accounts expected:
    /// 0. `[]` Config account
    /// 1. `[writable]` Lottery account
    /// 2. `[signer]` Authority (payer if VoteTally PDA is created)
    /// 3. `[]` System program
    /// 4. `[writable]` VoteTally PDA (`["vote_tally", lottery]`)
    ///    5..N. `[]` Participant accounts
    UploadReveals {
        /// Entries of (participant pubkey, plaintext reveal)
        entries: Vec<(solana_program::pubkey::Pubkey, Vec<u8>)>,
    },

    /// Adjust reveal window timing (emergency use)
    ///
    /// Authority can adjust reveal window timing in case of issues.
    /// Should be used sparingly and only when necessary.
    ///
    /// Accounts expected:
    /// 0. `[]` Config account
    /// 1. `[writable]` Lottery account
    /// 2. `[signer]` Authority
    // Adjust* instructions removed in favor of chain-clocked BeginRevealNow

    ///    Begin or extend reveal windows using chain clock and durations
    ///
    /// Authority-only. Uses Clock::get() on-chain. Optional durations override
    /// config defaults. Never shortens existing windows; only starts or extends.
    BeginRevealNow {
        /// Optional attestation window length in seconds; use config default when None/0
        attestation_secs: u32,
        /// Optional upload window length in seconds; use config default when None/0
        upload_secs: u32,
    },

    /// Finalize winners into a fixed root and bounded winner pages in chunks
    ///
    /// Aggregates every ticket buyer into a FinalizationLedger, then scans
    /// sorted participant chunks once per weighted draw round. Included reveals
    /// determine entropy and winner-count votes; every buyer remains eligible.
    ///
    /// Accounts expected:
    /// 0. `[]` Config account
    /// 1. `[writable]` Lottery account
    /// 2. `[writable]` Vault account
    /// 3. `[writable, signer]` Permissionless transaction payer
    /// 4. `[]` System program
    /// 5. `[writable]` FinalizationLedger PDA (`["finalization_root_v2", lottery]`)
    /// 6. `[writable]` WinnerPage PDA (`["winner_page", lottery, page_index]`)
    ///    7..N. `[writable]` Participant accounts in strict wallet order
    FinalizeWinners,

    /// Begin the reveal phase immediately (testing-only convenience)
    ///
    /// Authority-only. Sets the reveal window to start now and end after the
    /// standard duration. Only available in builds with `allow-early-upload`.
    /// Fails if lottery is settled or already within the reveal window.
    ///
    /// Accounts expected:
    /// 0. `[]` Config account
    /// 1. `[writable]` Lottery account
    /// 2. `[signer]` Authority
    BeginRevealPhase,

    /// Finalize lottery when no attestations were submitted
    ///
    /// Emits RefundsIssued event so participants can claim refunds on-chain.
    /// Can only be called after attestation deadline has passed.
    ///
    /// Accounts expected:
    /// 0. `[]` Config account
    /// 1. `[writable]` Lottery account
    /// 2. `[writable]` Vault account
    /// 3. `[writable]` Authority wallet (signer)
    FinalizeNoAttesters,

    /// Process a batch of winner payouts
    ///
    /// Verifies fixed winner-page entries and transfers funds directly to winners.
    /// Can be called multiple times to process all winners in batches.
    ///
    /// Accounts expected:
    /// 0. `[]` Config account
    /// 1. `[writable]` Lottery account
    /// 2. `[writable]` Vault account
    /// 3. `[writable]` Authority fee/rent recipient
    /// 4. `[]` System program
    /// 5. `[writable]` WinnerPage PDA (`["winner_page", lottery, page_index]`)
    ///    6..N. `[writable]` Winner wallet accounts
    SettlePayoutBatch {
        lottery_id: u64,
        batch_index: u32,
        winners: Vec<settle_payout_batch::WinnerProof>,
    },

    /// Legacy placeholder to preserve Borsh enum discriminants.
    ///
    /// Tag 11 is removed from manual dispatch and is no longer supported.
    SettlementBegin,

    /// Legacy placeholder to preserve Borsh enum discriminants.
    ///
    /// Tag 12 is removed from manual dispatch and is no longer supported.
    SettlementChunk,

    /// Legacy placeholder to preserve Borsh enum discriminants.
    ///
    /// Tag 13 is removed from manual dispatch and is no longer supported.
    SettlementFinalize,

    /// Claim a refund after a cancelled/no-attester lottery.
    ///
    /// Accounts expected:
    /// 0. `[]` Config account
    /// 1. `[writable]` Lottery account
    /// 2. `[writable]` Vault account
    /// 3. `[writable]` Participant account
    /// 4. `[writable, signer]` Participant wallet
    ClaimRefund,

    /// Permissionlessly attest and include a reveal on-chain.
    ///
    /// Accounts expected:
    /// 0. `[]` Config account
    /// 1. `[writable]` Lottery account
    /// 2. `[writable]` Participant account
    /// 3. `[signer]` Participant wallet
    AttestReveal {
        voted_number_of_winners: u64,
        reveal_plaintext: Vec<u8>,
    },

    /// Close a refund-path vault after all participant refunds are claimed.
    ///
    /// Accounts expected:
    /// 0. `[]` Config account
    /// 1. `[]` Lottery account
    /// 2. `[writable]` Vault account
    /// 3. `[writable]` Authority wallet
    CloseRefundVault,

    /// Close a terminal participant account and return its rent to the wallet.
    ///
    /// Accounts expected:
    /// 0. `[]` Config account
    /// 1. `[]` Lottery account
    /// 2. `[writable]` Participant account
    /// 3. `[writable, signer]` Participant wallet
    CloseParticipant,
}

/// Instruction processing dispatcher
///
/// Routes incoming instruction data to the appropriate handler function.
/// Each instruction type has its own dedicated processing module.
pub fn process_instruction(
    program_id: &solana_program::pubkey::Pubkey,
    accounts: &[solana_program::account_info::AccountInfo],
    instruction_data: &[u8],
) -> solana_program::entrypoint::ProgramResult {
    // Ensure we have at least one byte for the tag
    if instruction_data.is_empty() {
        solana_program::msg!("ENTRY: empty instruction data");
        return Err(crate::error::Error::InvalidInstruction.into());
    }

    let tag = instruction_data[0];
    solana_program::msg!("ENTRY tag={} len={}", tag, instruction_data.len());

    // Manual tag dispatch for stability - no enum deserialization
    match tag {
        TAG_INITIALIZE => {
            // Layout: [tag u8][ticket_price_lamports u64][service_charge_bps u16][max_winners_cap u32]
            if instruction_data.len() < 1 + 8 + 2 + 4 {
                return Err(crate::error::Error::InvalidInstruction.into());
            }
            let ticket_price_lamports = u64::from_le_bytes([
                instruction_data[1],
                instruction_data[2],
                instruction_data[3],
                instruction_data[4],
                instruction_data[5],
                instruction_data[6],
                instruction_data[7],
                instruction_data[8],
            ]);
            let service_charge_bps =
                u16::from_le_bytes([instruction_data[9], instruction_data[10]]);
            let max_winners_cap = u32::from_le_bytes([
                instruction_data[11],
                instruction_data[12],
                instruction_data[13],
                instruction_data[14],
            ]);
            process_initialize(
                program_id,
                accounts,
                ticket_price_lamports,
                service_charge_bps,
                max_winners_cap,
            )
        }

        TAG_UPDATE_SERVICE_CHARGE => {
            // Layout: [tag u8][new_bps u16]
            if instruction_data.len() < 1 + 2 {
                return Err(crate::error::Error::InvalidInstruction.into());
            }
            let new_bps = u16::from_le_bytes([instruction_data[1], instruction_data[2]]);
            process_update_service_charge(program_id, accounts, new_bps)
        }

        TAG_CREATE_LOTTERY => {
            // Layout: [tag u8]
            process_create_lottery(program_id, accounts)
        }

        TAG_BUY_TICKETS => {
            // Layout: [tag u8][option u8][hash? 32][tickets u64]
            if instruction_data.len() < 1 + 1 + 8 {
                return Err(crate::error::Error::InvalidInstruction.into());
            }
            let has_hash = instruction_data[1] != 0;
            let expected_len = if has_hash { 1 + 1 + 32 + 8 } else { 1 + 1 + 8 };
            if instruction_data.len() < expected_len {
                return Err(crate::error::Error::InvalidInstruction.into());
            }

            let (proof_of_chance_hash, tickets_offset) = if has_hash {
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&instruction_data[2..34]);
                (Some(hash), 34)
            } else {
                (None, 2)
            };

            let number_of_tickets = u64::from_le_bytes([
                instruction_data[tickets_offset],
                instruction_data[tickets_offset + 1],
                instruction_data[tickets_offset + 2],
                instruction_data[tickets_offset + 3],
                instruction_data[tickets_offset + 4],
                instruction_data[tickets_offset + 5],
                instruction_data[tickets_offset + 6],
                instruction_data[tickets_offset + 7],
            ]);

            process_buy_tickets(
                program_id,
                accounts,
                proof_of_chance_hash,
                number_of_tickets,
            )
        }

        TAG_ATTEST_UPLOADED => {
            // Layout: [tag u8][voted_number_of_winners u64]
            if instruction_data.len() < 1 + 8 {
                return Err(crate::error::Error::InvalidInstruction.into());
            }
            let voted_number_of_winners = u64::from_le_bytes([
                instruction_data[1],
                instruction_data[2],
                instruction_data[3],
                instruction_data[4],
                instruction_data[5],
                instruction_data[6],
                instruction_data[7],
                instruction_data[8],
            ]);
            process_attest_uploaded(program_id, accounts, voted_number_of_winners)
        }

        TAG_UPLOAD_REVEALS => {
            // Layout: [tag u8][Vec<(Pubkey, Vec<u8>)> borsh-encoded]
            if instruction_data.len() < 1 + 4 {
                return Err(crate::error::Error::InvalidInstruction.into());
            }
            let data = &instruction_data[1..];
            let entries = Vec::<(solana_program::pubkey::Pubkey, Vec<u8>)>::try_from_slice(data)
                .map_err(|_| crate::error::Error::InvalidInstruction)?;
            process_upload_reveals(program_id, accounts, entries)
        }

        TAG_BEGIN_REVEAL_NOW => {
            // Layout: [tag u8][attestation_secs u32][upload_secs u32]
            if instruction_data.len() < 1 + 4 + 4 {
                return Err(crate::error::Error::InvalidInstruction.into());
            }
            let attestation_secs = u32::from_le_bytes([
                instruction_data[1],
                instruction_data[2],
                instruction_data[3],
                instruction_data[4],
            ]);
            let upload_secs = u32::from_le_bytes([
                instruction_data[5],
                instruction_data[6],
                instruction_data[7],
                instruction_data[8],
            ]);
            process_begin_reveal_phase(program_id, accounts, attestation_secs, upload_secs)
        }

        TAG_FINALIZE_WINNERS => {
            // Layout: [tag u8]
            process_finalize_winners(program_id, accounts)
        }

        TAG_SETTLE_PAYOUT_BATCH => {
            // Layout: [tag u8][lottery_id u64][batch_index u32][winners Vec<WinnerProof>]
            if instruction_data.len() < 1 + 8 + 4 {
                return Err(crate::error::Error::InvalidInstruction.into());
            }

            let lottery_id = u64::from_le_bytes([
                instruction_data[1],
                instruction_data[2],
                instruction_data[3],
                instruction_data[4],
                instruction_data[5],
                instruction_data[6],
                instruction_data[7],
                instruction_data[8],
            ]);

            let batch_index = u32::from_le_bytes([
                instruction_data[9],
                instruction_data[10],
                instruction_data[11],
                instruction_data[12],
            ]);

            let winners_data = &instruction_data[13..];
            let winners = Vec::<settle_payout_batch::WinnerProof>::try_from_slice(winners_data)
                .map_err(|_| crate::error::Error::InvalidInstruction)?;

            process_settle_payout_batch(program_id, accounts, lottery_id, batch_index, winners)
        }

        TAG_BEGIN_REVEAL_PHASE => {
            // Layout: [tag u8]
            process_begin_reveal_phase(program_id, accounts, 0u32, 0u32)
        }

        TAG_FINALIZE_NO_ATTESTERS => {
            // Layout: [tag u8]
            process_finalize_no_attesters(program_id, accounts)
        }

        TAG_CLAIM_REFUND => {
            // Layout: [tag u8]
            process_claim_refund(program_id, accounts)
        }

        TAG_ATTEST_REVEAL => {
            // Layout: [tag u8][voted_number_of_winners u64][reveal_plaintext Vec<u8>]
            if instruction_data.len() < 1 + 8 + 4 {
                return Err(crate::error::Error::InvalidInstruction.into());
            }
            let voted_number_of_winners = u64::from_le_bytes([
                instruction_data[1],
                instruction_data[2],
                instruction_data[3],
                instruction_data[4],
                instruction_data[5],
                instruction_data[6],
                instruction_data[7],
                instruction_data[8],
            ]);
            let reveal_len = u32::from_le_bytes([
                instruction_data[9],
                instruction_data[10],
                instruction_data[11],
                instruction_data[12],
            ]) as usize;
            let reveal_end = 13usize
                .checked_add(reveal_len)
                .ok_or(crate::error::Error::MathOverflow)?;
            if instruction_data.len() < reveal_end {
                return Err(crate::error::Error::InvalidInstruction.into());
            }
            let reveal_plaintext = instruction_data[13..reveal_end].to_vec();
            process_attest_reveal(
                program_id,
                accounts,
                voted_number_of_winners,
                reveal_plaintext,
            )
        }

        TAG_CLOSE_REFUND_VAULT => {
            // Layout: [tag u8]
            process_close_refund_vault(program_id, accounts)
        }

        TAG_CLOSE_PARTICIPANT => {
            // Layout: [tag u8]
            process_close_participant(program_id, accounts)
        }

        _ => {
            solana_program::msg!("ENTRY: unknown tag {}", tag);
            Err(crate::error::Error::InvalidInstruction.into())
        }
    }
}
