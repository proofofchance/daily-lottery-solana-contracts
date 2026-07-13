//! # Settle Payout Batch Instruction
//!
//! This instruction processes a batch of winner payouts, verifying each entry
//! against its fixed winner-page PDA and transferring funds directly to winners.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery, Vault, WinnerPage, WINNERS_PER_PAGE},
    utils::{
        account::{read_account_data, write_account_data},
        pda::{assert_pda_owned, derive_winner_page_pda},
        validation::{compute_service_fee, require_key_match, require_writable},
    },
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::Sysvar,
};
use solana_system_interface::program as system_program;

/// Winner proof for batch settlement
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct WinnerProof {
    /// Winner index in the merkle tree
    pub index: u64,
    /// Recipient wallet address
    pub recipient: Pubkey,
    /// Payout amount in lamports
    pub amount: u64,
    /// Legacy ABI field. Protocol v2 verifies the fixed winner page directly.
    pub merkle_proof: Vec<[u8; 32]>,
}

/// Process a batch of winner payouts
pub fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    lottery_id: u64,
    batch_index: u32,
    winners: Vec<WinnerProof>,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    // Get core accounts
    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let vault_ai = next_account_info(account_info_iter)?;
    let authority_ai = next_account_info(account_info_iter)?;
    let system_program_ai = next_account_info(account_info_iter)?;
    let winner_page_ai = next_account_info(account_info_iter)?;

    // Remaining accounts are winner recipient wallets
    let recipient_accounts: Vec<&AccountInfo> = account_info_iter.collect();

    // Validate accounts
    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(lottery_ai)?;
    require_writable(vault_ai)?;
    require_writable(authority_ai)?;
    require_key_match(system_program_ai, &system_program::id())?;

    // Read account data
    let config: Config = read_account_data(config_ai)?;
    let mut lottery: Lottery = read_account_data(lottery_ai)?;
    let _vault: Vault = read_account_data(vault_ai)?;

    // Re-assert PDAs using loaded data to catch ordering or program-id mismatches
    assert_pda_owned(
        program_id,
        lottery_ai,
        &[
            b"lottery",
            config_ai.key.as_ref(),
            &lottery.id.to_le_bytes(),
        ],
    )?;
    assert_pda_owned(program_id, vault_ai, &[b"vault", lottery_ai.key.as_ref()])?;

    // Validate authority fee/rent recipient. The authority does not need to sign:
    // once the winners root is fixed, any caller may execute merkle-verified payouts.
    if authority_ai.key != &config.authority {
        return Err(Error::Unauthorized.into());
    }

    // Validate lottery
    if lottery.id != lottery_id {
        return Err(Error::InvalidInstruction.into());
    }

    if lottery.settlement_complete {
        return Ok(());
    }

    if lottery.winners_count == 0 {
        return Err(Error::InvalidInstruction.into());
    }

    if winners.is_empty() {
        return Err(Error::InvalidInstruction.into());
    }
    let page_index = u32::try_from(winners[0].index / WINNERS_PER_PAGE as u64)
        .map_err(|_| Error::MathOverflow)?;
    let (expected_page, _) = derive_winner_page_pda(program_id, lottery_ai.key, page_index);
    if expected_page != *winner_page_ai.key {
        return Err(Error::InvalidSeeds.into());
    }
    assert_pda_owned(
        program_id,
        winner_page_ai,
        &[
            b"winner_page",
            lottery_ai.key.as_ref(),
            &page_index.to_le_bytes(),
        ],
    )?;
    require_writable(winner_page_ai)?;
    let winner_page: WinnerPage = read_account_data(winner_page_ai)?;
    if winner_page.lottery != *lottery_ai.key || winner_page.page_index != page_index {
        return Err(Error::InvalidAccountData.into());
    }
    let expected_amount = lottery.total_payout / lottery.winners_count;

    // Validate we have enough recipient accounts
    if recipient_accounts.len() < winners.len() {
        return Err(Error::MissingAccount.into());
    }

    let clock = solana_program::clock::Clock::get()?;

    // Process each winner in the batch
    for (i, winner) in winners.iter().enumerate() {
        // 0. Validate index is within range (early fail with context)
        if winner.index >= lottery.winners_count {
            solana_program::msg!(
                "PAYOUT err: winner.index {} >= winners_count {} (i={})",
                winner.index,
                lottery.winners_count,
                i
            );
            return Err(Error::InvalidInstruction.into());
        }

        let winner_page_index = winner.index / WINNERS_PER_PAGE as u64;
        if winner_page_index != page_index as u64 || winner.amount != expected_amount {
            return Err(Error::InvalidInstruction.into());
        }
        let page_offset = (winner.index % WINNERS_PER_PAGE as u64) as usize;
        let (stored, page_paid) = {
            let page_data = winner_page_ai.try_borrow_data()?;
            (
                winner_page
                    .winner(&page_data, page_offset)
                    .ok_or(Error::InvalidInstruction)?,
                winner_page.is_paid(&page_data, page_offset),
            )
        };
        if stored.wallet != winner.recipient {
            return Err(Error::InvalidInstruction.into());
        }

        let lottery_paid = lottery.is_winner_paid(winner.index);
        if page_paid || lottery_paid {
            if page_paid && lottery_paid {
                continue;
            }
            return Err(Error::InvalidAccountData.into());
        }

        // 2. Check idempotency - ensure winner hasn't been paid already
        // 3. Find recipient account
        let recipient_ai = recipient_accounts.get(i).ok_or(Error::MissingAccount)?;

        if recipient_ai.key != &winner.recipient {
            return Err(Error::InvalidAccount.into());
        }

        // 4. Transfer funds directly from vault to recipient
        if winner.amount > 0 {
            let vault_lamports_ref = &mut **vault_ai.try_borrow_mut_lamports()?;
            let recipient_lamports_ref = &mut **recipient_ai.try_borrow_mut_lamports()?;

            if *vault_lamports_ref < winner.amount {
                return Err(Error::InsufficientFunds.into());
            }

            *vault_lamports_ref = vault_lamports_ref.saturating_sub(winner.amount);
            *recipient_lamports_ref = recipient_lamports_ref.saturating_add(winner.amount);
        }

        // 5. Mark winner as paid
        {
            let mut page_data = winner_page_ai.try_borrow_mut_data()?;
            winner_page.mark_paid(&mut page_data, page_offset)?;
        }
        lottery.mark_winner_paid(winner.index)?;

        // 6. Emit winner paid event
        let event = LotteryEvent::WinnerPaid {
            lottery_id: lottery.id,
            lottery: lottery_ai.key.to_string(),
            winner: winner.recipient.to_string(),
            amount: winner.amount,
            batch_index,
            winner_index: winner.index,
            timestamp: clock.unix_timestamp,
        };
        event.emit();
    }

    // Update batch counter
    lottery.increment_settlement_batch();

    // Check if all winners have been paid
    if lottery.all_winners_paid() {
        // Calculate and transfer service fee to authority
        let service_fee = compute_service_fee(lottery.total_funds, lottery.service_charge_bps)?;
        let payout_remainder = lottery
            .total_funds
            .saturating_sub(lottery.total_payout)
            .saturating_sub(service_fee);

        // Transfer service fee to authority
        if service_fee > 0 {
            let vault_lamports_ref = &mut **vault_ai.try_borrow_mut_lamports()?;
            let authority_lamports_ref = &mut **authority_ai.try_borrow_mut_lamports()?;

            if *vault_lamports_ref < service_fee {
                return Err(Error::InsufficientFunds.into());
            }

            *vault_lamports_ref = vault_lamports_ref.saturating_sub(service_fee);
            *authority_lamports_ref = authority_lamports_ref.saturating_add(service_fee);
        }

        // Transfer any remainder to authority (from division rounding)
        if payout_remainder > 0 {
            let vault_lamports_ref = &mut **vault_ai.try_borrow_mut_lamports()?;
            let authority_lamports_ref = &mut **authority_ai.try_borrow_mut_lamports()?;

            if *vault_lamports_ref < payout_remainder {
                return Err(Error::InsufficientFunds.into());
            }

            *vault_lamports_ref = vault_lamports_ref.saturating_sub(payout_remainder);
            *authority_lamports_ref = authority_lamports_ref.saturating_add(payout_remainder);
        }

        // Close vault and reclaim rent to authority
        let vault_lamports_ref = &mut **vault_ai.try_borrow_mut_lamports()?;
        let authority_lamports_ref = &mut **authority_ai.try_borrow_mut_lamports()?;
        let remaining_rent = *vault_lamports_ref;

        if remaining_rent > 0 {
            *authority_lamports_ref = authority_lamports_ref.saturating_add(remaining_rent);
            *vault_lamports_ref = 0;
        }

        lottery.complete_settlement()?;

        // Emit settlement complete event
        let event = LotteryEvent::PayoutsComplete {
            lottery_id: lottery.id,
            lottery: lottery_ai.key.to_string(),
            total_winners: lottery.winners_count,
            total_paid: lottery.total_payout,
            batches_completed: lottery.settlement_batches_completed,
            timestamp: clock.unix_timestamp,
        };
        event.emit();

        // Emit service fee paid event
        if service_fee > 0 || payout_remainder > 0 {
            let event = LotteryEvent::ServiceFeePaid {
                lottery_id: lottery.id,
                lottery: lottery_ai.key.to_string(),
                authority: authority_ai.key.to_string(),
                service_fee,
                remainder: payout_remainder,
                vault_rent_reclaimed: remaining_rent,
                timestamp: clock.unix_timestamp,
            };
            event.emit();
        }
    }

    // Write updated lottery data
    write_account_data(lottery_ai, "Lottery", &lottery)?;

    Ok(())
}
