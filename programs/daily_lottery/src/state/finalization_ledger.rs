//! Fixed-size protocol-v2 finalization state for daily lottery rounds.
//!
//! Winner records live in bounded `WinnerPage` PDAs, so neither finalization nor
//! payout requires an account whose size grows with the configured winner cap.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

pub const FINALIZATION_PROTOCOL_VERSION: u16 = 2;
pub const FINALIZATION_PHASE_AGGREGATING: u8 = 0;
pub const FINALIZATION_PHASE_SELECTING: u8 = 1;
pub const FINALIZATION_PHASE_COMPLETED: u8 = 2;
pub const WINNERS_PER_PAGE: usize = 100;

#[derive(BorshSerialize, BorshDeserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SelectedWinner {
    pub wallet: Pubkey,
    pub tickets: u64,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct FinalizationLedger {
    pub lottery: Pubkey,
    pub protocol_version: u16,
    pub phase: u8,
    pub generation: u32,
    pub required_count: u64,
    pub processed_count: u64,
    pub eligible_count: u64,
    pub total_eligible_tickets: u64,
    pub seed: [u8; 32],
    pub participants_commitment: [u8; 32],
    pub target_winners: u32,
    pub current_round: u32,
    pub selected_count: u32,
    pub selected_tickets_total: u64,
    pub round_processed_count: u64,
    pub round_remaining_tickets_seen: u64,
    pub round_draw_index: u64,
    pub pending_winner: Pubkey,
    pub pending_winner_tickets: u64,
    pub pending_winner_found: bool,
    pub has_last_wallet: bool,
    pub last_processed_wallet: Pubkey,
    pub winners_commitment: [u8; 32],
    pub completed: bool,
    pub started_at_unix: i64,
    pub completed_at_unix: i64,
    pub reserved: [u8; 64],
}

impl FinalizationLedger {
    pub const SIZE: usize = 8 + // discriminator
        32 + 2 + 1 + 4 + 8 + 8 + 8 + 8 + 32 + 32 + 4 + 4 + 4 + 8 +
        8 + 8 + 8 + 32 + 8 + 1 + 1 + 32 + 32 + 1 + 8 + 8 + 64;

    pub fn size_for(_max_winners: usize) -> usize {
        Self::SIZE
    }

    pub fn new(lottery: Pubkey, required_count: u64, started_at_unix: i64) -> Self {
        Self {
            lottery,
            protocol_version: FINALIZATION_PROTOCOL_VERSION,
            phase: FINALIZATION_PHASE_AGGREGATING,
            generation: 1,
            required_count,
            processed_count: 0,
            eligible_count: 0,
            total_eligible_tickets: 0,
            seed: [0; 32],
            participants_commitment: [0; 32],
            target_winners: 0,
            current_round: 0,
            selected_count: 0,
            selected_tickets_total: 0,
            round_processed_count: 0,
            round_remaining_tickets_seen: 0,
            round_draw_index: 0,
            pending_winner: Pubkey::default(),
            pending_winner_tickets: 0,
            pending_winner_found: false,
            has_last_wallet: false,
            last_processed_wallet: Pubkey::default(),
            winners_commitment: [0; 32],
            completed: false,
            started_at_unix,
            completed_at_unix: 0,
            reserved: [0; 64],
        }
    }

    pub fn reset_pass_cursor(&mut self) {
        self.has_last_wallet = false;
        self.last_processed_wallet = Pubkey::default();
    }

    pub fn begin_selection_round(&mut self, draw_index: u64) {
        self.phase = FINALIZATION_PHASE_SELECTING;
        self.round_processed_count = 0;
        self.round_remaining_tickets_seen = 0;
        self.round_draw_index = draw_index;
        self.pending_winner = Pubkey::default();
        self.pending_winner_tickets = 0;
        self.pending_winner_found = false;
        self.reset_pass_cursor();
    }

    pub fn remaining_tickets(&self) -> Result<u64, crate::error::Error> {
        self.total_eligible_tickets
            .checked_sub(self.selected_tickets_total)
            .ok_or(crate::error::Error::MathOverflow)
    }

    pub fn record_winner(&mut self, winner: SelectedWinner) -> Result<(), crate::error::Error> {
        if self.selected_count >= self.target_winners {
            return Err(crate::error::Error::InvalidInstruction);
        }
        self.selected_count = self
            .selected_count
            .checked_add(1)
            .ok_or(crate::error::Error::MathOverflow)?;
        self.selected_tickets_total = self
            .selected_tickets_total
            .checked_add(winner.tickets)
            .ok_or(crate::error::Error::MathOverflow)?;
        self.winners_commitment =
            extend_winners_commitment(self.winners_commitment, self.selected_count - 1, &winner);
        Ok(())
    }

    pub fn complete(&mut self, timestamp: i64) {
        self.phase = FINALIZATION_PHASE_COMPLETED;
        self.completed = true;
        self.completed_at_unix = timestamp;
        self.reset_pass_cursor();
    }
}

/// Small serialized header for a fixed-size winner-page account.
///
/// Winner entries and the paid bitmap are stored in the trailing account bytes
/// and accessed by offset. Keeping the large fixed arrays out of this Rust value
/// prevents BPF deserialization from placing a multi-kilobyte value on the stack.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct WinnerPage {
    pub lottery: Pubkey,
    pub generation: u32,
    pub page_index: u32,
    pub count: u16,
    pub reserved: [u8; 64],
}

impl Default for WinnerPage {
    fn default() -> Self {
        Self {
            lottery: Pubkey::default(),
            generation: 0,
            page_index: 0,
            count: 0,
            reserved: [0; 64],
        }
    }
}

impl WinnerPage {
    pub const SELECTED_WINNER_SIZE: usize = 32 + 8;
    pub const HEADER_SIZE: usize = 8 + 32 + 4 + 4 + 2 + 64;
    pub const ENTRIES_OFFSET: usize = Self::HEADER_SIZE;
    pub const BITMAP_OFFSET: usize =
        Self::ENTRIES_OFFSET + (WINNERS_PER_PAGE * Self::SELECTED_WINNER_SIZE);
    pub const BITMAP_SIZE: usize = WINNERS_PER_PAGE.div_ceil(8);
    pub const SIZE: usize = Self::BITMAP_OFFSET + Self::BITMAP_SIZE;

    pub fn new(lottery: Pubkey, generation: u32, page_index: u32) -> Self {
        Self {
            lottery,
            generation,
            page_index,
            ..Self::default()
        }
    }

    pub fn append(
        &mut self,
        account_data: &mut [u8],
        winner: SelectedWinner,
    ) -> Result<u16, crate::error::Error> {
        let offset = self.count as usize;
        if offset >= WINNERS_PER_PAGE || account_data.len() < Self::SIZE {
            return Err(crate::error::Error::InvalidInstruction);
        }
        let start = Self::entry_offset(offset);
        account_data[start..start + 32].copy_from_slice(winner.wallet.as_ref());
        account_data[start + 32..start + 40].copy_from_slice(&winner.tickets.to_le_bytes());
        self.count = self
            .count
            .checked_add(1)
            .ok_or(crate::error::Error::MathOverflow)?;
        Ok(offset as u16)
    }

    pub fn winner(&self, account_data: &[u8], offset: usize) -> Option<SelectedWinner> {
        if offset >= self.count as usize || account_data.len() < Self::SIZE {
            return None;
        }
        let start = Self::entry_offset(offset);
        let mut wallet = [0u8; 32];
        wallet.copy_from_slice(&account_data[start..start + 32]);
        let mut tickets = [0u8; 8];
        tickets.copy_from_slice(&account_data[start + 32..start + 40]);
        Some(SelectedWinner {
            wallet: Pubkey::new_from_array(wallet),
            tickets: u64::from_le_bytes(tickets),
        })
    }

    pub fn is_paid(&self, account_data: &[u8], offset: usize) -> bool {
        offset < self.count as usize
            && account_data.len() >= Self::SIZE
            && (account_data[Self::BITMAP_OFFSET + offset / 8] & (1 << (offset % 8))) != 0
    }

    pub fn mark_paid(
        &self,
        account_data: &mut [u8],
        offset: usize,
    ) -> Result<(), crate::error::Error> {
        if offset >= self.count as usize
            || account_data.len() < Self::SIZE
            || self.is_paid(account_data, offset)
        {
            return Err(crate::error::Error::WinnerAlreadyPaid);
        }
        account_data[Self::BITMAP_OFFSET + offset / 8] |= 1 << (offset % 8);
        Ok(())
    }

    const fn entry_offset(offset: usize) -> usize {
        Self::ENTRIES_OFFSET + (offset * Self::SELECTED_WINNER_SIZE)
    }
}

fn extend_winners_commitment(current: [u8; 32], index: u32, winner: &SelectedWinner) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"IKIGAI_WINNERS_V2");
    h.update(current);
    h.update(index.to_le_bytes());
    h.update(winner.wallet.to_bytes());
    h.update(winner.tickets.to_le_bytes());
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn below_eight_kib(size: usize) -> bool {
        size < 8 * 1024
    }

    #[test]
    fn fixed_accounts_cover_one_thousand_winners_below_eight_kib() {
        assert_eq!(
            FinalizationLedger::size_for(1),
            FinalizationLedger::size_for(1_000)
        );
        assert!(below_eight_kib(FinalizationLedger::SIZE));
        assert!(below_eight_kib(WinnerPage::SIZE));
        assert_eq!(1_000usize.div_ceil(WINNERS_PER_PAGE), 10);

        let mut page = WinnerPage::new(Pubkey::new_unique(), 1, 0);
        let mut data = vec![0u8; WinnerPage::SIZE];
        for _ in 0..WINNERS_PER_PAGE {
            page.append(
                &mut data,
                SelectedWinner {
                    wallet: Pubkey::new_unique(),
                    tickets: 1,
                },
            )
            .unwrap();
        }
        assert_eq!(page.count as usize, WINNERS_PER_PAGE);
        assert!(page.append(&mut data, SelectedWinner::default()).is_err());
    }
}
