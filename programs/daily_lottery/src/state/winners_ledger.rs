//! WinnersLedger PDA - tracks paid winners bitmap and batch progress

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

#[derive(BorshSerialize, BorshDeserialize, Debug, Default, Clone)]
pub struct WinnersLedger {
    /// Lottery this ledger belongs to
    pub lottery: Pubkey,
    /// Total winners for this lottery
    pub winners_count: u64,
    /// Bitmap of paid winners (bit i set means paid)
    pub paid_bitmap: Vec<u8>,
    /// Number of payout batches completed
    pub settlement_batches_completed: u32,
}

impl WinnersLedger {
    pub fn size_for(winners_count: u64) -> usize {
        let bytes_needed = (winners_count as usize).div_ceil(8).max(1);
        // discriminator 8 + lottery[32] + winners_count[8] + vec len[4] + bytes + batches[4]
        8 + 32 + 8 + 4 + bytes_needed + 4
    }

    pub fn is_winner_paid(&self, winner_index: u64) -> bool {
        if winner_index >= self.winners_count {
            return false;
        }
        let byte_index = (winner_index / 8) as usize;
        let bit_index = (winner_index % 8) as u8;
        if byte_index >= self.paid_bitmap.len() {
            return false;
        }
        (self.paid_bitmap[byte_index] >> bit_index) & 1 == 1
    }

    pub fn mark_winner_paid(&mut self, winner_index: u64) -> Result<(), crate::error::Error> {
        if winner_index >= self.winners_count {
            return Err(crate::error::Error::InvalidInstruction);
        }
        let byte_index = (winner_index / 8) as usize;
        let bit_index = (winner_index % 8) as u8;
        while byte_index >= self.paid_bitmap.len() {
            self.paid_bitmap.push(0);
        }
        self.paid_bitmap[byte_index] |= 1 << bit_index;
        Ok(())
    }

    pub fn all_winners_paid(&self) -> bool {
        for i in 0..self.winners_count {
            if !self.is_winner_paid(i) {
                return false;
            }
        }
        true
    }
}
