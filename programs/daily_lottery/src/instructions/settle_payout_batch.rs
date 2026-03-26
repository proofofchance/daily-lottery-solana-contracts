//! # Settle Payout Batch Instruction
//!
//! This instruction processes a batch of winner payouts, verifying merkle proofs
//! and transferring funds directly to winners.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery, Vault, WinnersLedger},
    utils::{
        account::{read_account_data, write_account_data},
        pda::{assert_pda_owned, derive_winners_ledger_pda},
        validation::{compute_service_fee, require_key_match, require_signer, require_writable},
    },
};
use borsh::{BorshDeserialize, BorshSerialize};
use sha2::{Digest, Sha256};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::Sysvar,
};
use solana_system_interface::{instruction as system_instruction, program as system_program};

/// Winner proof for batch settlement
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct WinnerProof {
    /// Winner index in the merkle tree
    pub index: u64,
    /// Recipient wallet address
    pub recipient: Pubkey,
    /// Payout amount in lamports
    pub amount: u64,
    /// Merkle proof path (array of sibling hashes)
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
    let winners_ledger_ai = next_account_info(account_info_iter)?;

    // Remaining accounts are winner recipient wallets
    let recipient_accounts: Vec<&AccountInfo> = account_info_iter.collect();

    // DEBUG: log core accounts and lengths to diagnose PDA issues at runtime
    solana_program::msg!(
        "PAYOUT dbg: accounts cfg={} lot={} vault={} auth={}",
        config_ai.key,
        lottery_ai.key,
        vault_ai.key,
        authority_ai.key
    );
    solana_program::msg!(
        "PAYOUT dbg: data_len cfg={} lot={} vault={}",
        config_ai.data.borrow().len(),
        lottery_ai.data.borrow().len(),
        vault_ai.data.borrow().len()
    );

    // Validate accounts
    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(lottery_ai)?;
    require_writable(vault_ai)?;
    require_signer(authority_ai)?;
    require_writable(authority_ai)?;
    require_key_match(system_program_ai, &system_program::id())?;

    // Read account data
    let config: Config = read_account_data(config_ai)?;
    let mut lottery: Lottery = read_account_data(lottery_ai)?;
    let _vault: Vault = read_account_data(vault_ai)?;

    // Ensure WinnersLedger exists and matches PDA; create if missing
    let (expected_ledger_pda, ledger_bump) = derive_winners_ledger_pda(program_id, lottery_ai.key);
    solana_program::msg!(
        "PAYOUT dbg: winners_ledger passed={} expected={} owner={} empty={}",
        winners_ledger_ai.key,
        expected_ledger_pda,
        winners_ledger_ai.owner,
        winners_ledger_ai.data_is_empty()
    );
    if &expected_ledger_pda != winners_ledger_ai.key {
        return Err(Error::InvalidSeeds.into());
    }
    // If uninitialized, allocate (owner may be SystemProgram here)
    if winners_ledger_ai.data_is_empty() {
        let ledger_space = WinnersLedger::size_for(lottery.winners_count);
        let lamports = solana_program::rent::Rent::get()?.minimum_balance(ledger_space);
        let create_ix = system_instruction::create_account(
            authority_ai.key,
            &expected_ledger_pda,
            lamports,
            ledger_space as u64,
            program_id,
        );
        solana_program::program::invoke_signed(
            &create_ix,
            &[
                authority_ai.clone(),
                winners_ledger_ai.clone(),
                system_program_ai.clone(),
            ],
            &[&[b"winners_ledger", lottery_ai.key.as_ref(), &[ledger_bump]]],
        )
        .map_err(|_| Error::InvalidInstruction)?;
        solana_program::msg!(
            "PAYOUT dbg: winners_ledger created space={} lamports={}",
            ledger_space,
            lamports
        );
        // Initialize content
        let ledger = WinnersLedger {
            lottery: *lottery_ai.key,
            winners_count: lottery.winners_count,
            paid_bitmap: vec![0u8; (lottery.winners_count as usize).div_ceil(8).max(1)],
            settlement_batches_completed: 0,
        };
        write_account_data(winners_ledger_ai, "WinnersLedger", &ledger)?;
        solana_program::msg!("PAYOUT dbg: winners_ledger initialized");
    } else {
        // Existing ledger must be owned by this program
        if winners_ledger_ai.owner != program_id {
            return Err(Error::IncorrectOwner.into());
        }
    }
    let mut ledger: WinnersLedger = read_account_data(winners_ledger_ai)?;

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

    // Validate authority
    if authority_ai.key != &config.authority {
        return Err(Error::Unauthorized.into());
    }

    // Validate lottery
    if lottery.id != lottery_id {
        return Err(Error::InvalidInstruction.into());
    }

    if lottery.settlement_complete {
        return Err(Error::LotteryAlreadySettled.into());
    }

    if lottery.winners_count == 0 {
        return Err(Error::InvalidInstruction.into());
    }

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

        // 1. Verify merkle proof
        verify_winner_merkle_proof(
            &lottery.winners_merkle_root,
            winner.index,
            &winner.recipient,
            winner.amount,
            &winner.merkle_proof,
        )?;

        // 2. Check idempotency - ensure winner hasn't been paid already
        if ledger.is_winner_paid(winner.index) {
            return Err(Error::WinnerAlreadyPaid.into());
        }

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
        ledger.mark_winner_paid(winner.index)?;
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
    ledger.settlement_batches_completed = ledger.settlement_batches_completed.saturating_add(1);

    // Check if all winners have been paid
    if ledger.all_winners_paid() {
        // Calculate and transfer service fee to authority
        let service_fee = compute_service_fee(lottery.total_funds, config.service_charge_bps)?;
        let payout_remainder = lottery
            .total_funds
            .saturating_sub(lottery.total_payout)
            .saturating_sub(service_fee);

        // Transfer service fee to authority
        if service_fee > 0 {
            let vault_lamports_ref = &mut **vault_ai.try_borrow_mut_lamports()?;
            let authority_lamports_ref = &mut **authority_ai.try_borrow_mut_lamports()?;

            if *vault_lamports_ref >= service_fee {
                *vault_lamports_ref = vault_lamports_ref.saturating_sub(service_fee);
                *authority_lamports_ref = authority_lamports_ref.saturating_add(service_fee);
            }
        }

        // Transfer any remainder to authority (from division rounding)
        if payout_remainder > 0 {
            let vault_lamports_ref = &mut **vault_ai.try_borrow_mut_lamports()?;
            let authority_lamports_ref = &mut **authority_ai.try_borrow_mut_lamports()?;

            if *vault_lamports_ref >= payout_remainder {
                *vault_lamports_ref = vault_lamports_ref.saturating_sub(payout_remainder);
                *authority_lamports_ref = authority_lamports_ref.saturating_add(payout_remainder);
            }
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
    write_account_data(winners_ledger_ai, "WinnersLedger", &ledger)?;

    Ok(())
}

/// Verify a winner's merkle proof against the stored root
fn verify_winner_merkle_proof(
    merkle_root: &[u8; 32],
    index: u64,
    recipient: &Pubkey,
    amount: u64,
    proof: &[[u8; 32]],
) -> Result<(), Error> {
    // Compute leaf hash: hash(index || recipient || amount)
    let mut leaf_data = Vec::new();
    leaf_data.extend_from_slice(&index.to_le_bytes());
    leaf_data.extend_from_slice(&recipient.to_bytes());
    leaf_data.extend_from_slice(&amount.to_le_bytes());

    let mut current_hash = Sha256::digest(&leaf_data);
    let mut current_hash_array = [0u8; 32];
    current_hash_array.copy_from_slice(&current_hash);

    // Traverse up the merkle tree using the proof
    let mut path_index = index;
    for sibling in proof {
        let mut combined = Vec::new();

        if path_index % 2 == 0 {
            // Current hash is left child
            combined.extend_from_slice(&current_hash_array);
            combined.extend_from_slice(sibling);
        } else {
            // Current hash is right child
            combined.extend_from_slice(sibling);
            combined.extend_from_slice(&current_hash_array);
        }

        current_hash = Sha256::digest(&combined);
        current_hash_array.copy_from_slice(&current_hash);
        path_index /= 2;
    }

    // Verify the computed root matches the stored root
    if current_hash_array == *merkle_root {
        Ok(())
    } else {
        Err(Error::InvalidMerkleProof)
    }
}
