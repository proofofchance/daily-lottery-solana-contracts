//! # Participant Account State
//!
//! The Participant account tracks an individual participant's involvement in a specific lottery,
//! including their proof-of-chance hash, ticket count, and attestation status.
//!
//! ## Key Features
//! - One participant account per wallet per lottery
//! - Immutable proof-of-chance hash (set on first ticket purchase)
//! - Cumulative ticket tracking across multiple purchases
//! - Attestation tracking for anti-censorship mechanism

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// Participant state for a specific lottery
///
/// Each participant has one account per lottery they participate in.
/// The account is created on their first ticket purchase and updated
/// with subsequent purchases and attestations.
///
/// ## PDA Seeds
/// `["participant", lottery_pubkey, wallet_pubkey]`
///
/// ## Proof-of-Chance Model
/// - Participant provides SHA256 hash of their secret phrase on first purchase
/// - Hash is immutable once set (ensures consistency across purchases)
/// - Plaintext is revealed off-chain during reveal window
/// - On-chain attestation proves off-chain upload to prevent censorship
#[derive(BorshSerialize, BorshDeserialize, Debug, Default, Clone)]
pub struct Participant {
    /// The lottery this participant is involved in
    pub lottery: Pubkey,

    /// The participant's wallet address
    pub wallet: Pubkey,

    /// SHA256 hash of the participant's proof-of-chance secret phrase
    /// Set on first ticket purchase and cannot be changed
    pub proof_of_chance_hash: [u8; 32],

    /// Total number of tickets purchased by this participant
    /// Accumulated across multiple purchase transactions
    pub tickets_bought: u64,

    /// Whether the participant has attested to uploading their reveal
    /// Used for anti-censorship: provider must include all attested participants
    pub attested_uploaded: bool,

    /// Unix timestamp when the attestation was submitted
    /// Used for tracking and potential dispute resolution
    pub attested_at_unix: i64,

    /// Participant's vote for the number of winners (submitted during attestation)
    /// 0 means not yet voted; otherwise 1..=participants_count-1 at time of attestation
    pub voted_number_of_winners: u64,

    /// Score derived from revealed lucky words (character count after lowercase+trim)
    /// Set during UploadReveals; informational only (not used for payout entropy)
    pub reveal_score: u64,
}

impl Participant {
    const REVEAL_INCLUDED_FLAG: u64 = 1 << 63;
    const SETTLEMENT_INCLUDED_FLAG: u64 = 1 << 62;

    /// Creates a new participant with their first ticket purchase
    pub fn new(
        lottery: Pubkey,
        wallet: Pubkey,
        proof_of_chance_hash: [u8; 32],
        initial_tickets: u64,
    ) -> Self {
        Self {
            lottery,
            wallet,
            proof_of_chance_hash,
            tickets_bought: initial_tickets,
            attested_uploaded: false,
            attested_at_unix: 0,
            voted_number_of_winners: 0,
            reveal_score: 0,
        }
    }

    /// Checks if this is a new participant (no tickets purchased yet)
    pub fn is_new(&self) -> bool {
        self.tickets_bought == 0
    }

    /// Adds more tickets to this participant's total
    pub fn add_tickets(&mut self, count: u64) -> Result<(), crate::error::Error> {
        self.tickets_bought = self
            .tickets_bought
            .checked_add(count)
            .ok_or(crate::error::Error::MathOverflow)?;
        Ok(())
    }

    /// Validates that a provided proof hash matches the stored one
    /// Used for subsequent ticket purchases to ensure consistency
    pub fn validate_proof_hash(&self, provided_hash: [u8; 32]) -> bool {
        self.proof_of_chance_hash == provided_hash
    }

    /// Sets the participant's proof-of-chance hash (only for new participants)
    pub fn set_proof_hash(&mut self, hash: [u8; 32]) -> Result<(), crate::error::Error> {
        if !self.is_new() {
            return Err(crate::error::Error::InvalidInstruction);
        }
        self.proof_of_chance_hash = hash;
        Ok(())
    }

    /// Marks the participant as having attested their reveal upload
    pub fn attest_upload(&mut self, timestamp: i64) -> Result<(), crate::error::Error> {
        if self.attested_uploaded {
            return Err(crate::error::Error::InvalidInstruction);
        }

        self.attested_uploaded = true;
        self.attested_at_unix = timestamp;
        Ok(())
    }

    /// Records the participant's vote for number of winners at attestation time
    pub fn set_vote_number_of_winners(
        &mut self,
        voted_count: u64,
    ) -> Result<(), crate::error::Error> {
        if self.attested_uploaded {
            // Disallow changing vote after attestation
            return Err(crate::error::Error::InvalidInstruction);
        }
        self.voted_number_of_winners =
            voted_count & !(Self::REVEAL_INCLUDED_FLAG | Self::SETTLEMENT_INCLUDED_FLAG);
        Ok(())
    }

    /// Returns the recorded vote without the reveal-included flag.
    pub fn voted_winners(&self) -> u64 {
        self.voted_number_of_winners
            & !(Self::REVEAL_INCLUDED_FLAG | Self::SETTLEMENT_INCLUDED_FLAG)
    }

    /// Returns true if this participant's reveal has already been processed.
    pub fn reveal_included(&self) -> bool {
        (self.voted_number_of_winners & Self::REVEAL_INCLUDED_FLAG) != 0
    }

    /// Marks the participant's reveal as processed (idempotent).
    pub fn mark_reveal_included(&mut self) {
        self.voted_number_of_winners |= Self::REVEAL_INCLUDED_FLAG;
    }

    /// Returns true if this participant has been processed during settlement.
    pub fn settlement_included(&self) -> bool {
        (self.voted_number_of_winners & Self::SETTLEMENT_INCLUDED_FLAG) != 0
    }

    /// Marks the participant as processed during settlement (idempotent).
    pub fn mark_settlement_included(&mut self) {
        self.voted_number_of_winners |= Self::SETTLEMENT_INCLUDED_FLAG;
    }

    /// Checks if the participant has any tickets in this lottery
    pub fn has_tickets(&self) -> bool {
        self.tickets_bought > 0
    }

    /// Gets the participant's ticket range for winner selection
    /// Returns (start_index, end_index) where end is exclusive
    pub fn get_ticket_range(&self, previous_total: u64) -> (u64, u64) {
        let start = previous_total;
        let end = previous_total + self.tickets_bought;
        (start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;

    #[test]
    fn test_new_participant() {
        let lottery = Pubkey::new_unique();
        let wallet = Pubkey::new_unique();
        let proof_hash = [1u8; 32];

        let participant = Participant::new(lottery, wallet, proof_hash, 5);

        assert_eq!(participant.lottery, lottery);
        assert_eq!(participant.wallet, wallet);
        assert_eq!(participant.proof_of_chance_hash, proof_hash);
        assert_eq!(participant.tickets_bought, 5);
        assert!(!participant.attested_uploaded);
        assert!(!participant.is_new());
        assert!(participant.has_tickets());
    }

    #[test]
    fn test_ticket_addition() {
        let mut participant = Participant {
            tickets_bought: 3,
            ..Default::default()
        };

        participant.add_tickets(2).unwrap();
        assert_eq!(participant.tickets_bought, 5);

        participant.add_tickets(0).unwrap();
        assert_eq!(participant.tickets_bought, 5);
    }

    #[test]
    fn test_proof_hash_validation() {
        let mut participant = Participant {
            tickets_bought: 0,
            ..Default::default()
        };
        let hash1 = [1u8; 32];
        let hash2 = [2u8; 32];

        // Can set hash for new participant
        participant.set_proof_hash(hash1).unwrap();
        assert_eq!(participant.proof_of_chance_hash, hash1);

        // Cannot change hash for existing participant
        participant.add_tickets(1).unwrap(); // No longer new
        assert!(participant.set_proof_hash(hash2).is_err());

        // Validation works correctly
        assert!(participant.validate_proof_hash(hash1));
        assert!(!participant.validate_proof_hash(hash2));
    }

    #[test]
    fn test_attestation() {
        let mut participant = Participant::default();
        let timestamp = 1000;

        // Can attest once
        participant.attest_upload(timestamp).unwrap();
        assert!(participant.attested_uploaded);
        assert_eq!(participant.attested_at_unix, timestamp);

        // Cannot attest twice
        assert!(participant.attest_upload(timestamp + 100).is_err());
    }

    #[test]
    fn test_ticket_range() {
        let participant = Participant {
            tickets_bought: 5,
            ..Default::default()
        };

        let (start, end) = participant.get_ticket_range(10);
        assert_eq!(start, 10);
        assert_eq!(end, 15);

        let (start, end) = participant.get_ticket_range(0);
        assert_eq!(start, 0);
        assert_eq!(end, 5);
    }

    #[test]
    fn test_settlement_flag_masks_vote() {
        let mut participant = Participant::default();
        participant.set_vote_number_of_winners(3).unwrap();
        participant.mark_reveal_included();
        participant.mark_settlement_included();

        assert_eq!(participant.voted_winners(), 3);
        assert!(participant.reveal_included());
        assert!(participant.settlement_included());
    }
}
