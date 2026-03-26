//! # Finalize No Attesters/No Reveals Instruction
//!
//! Finalizes a lottery when, by upload deadline, either (a) no attestations were
//! submitted, or (b) some attestations exist but zero reveals were uploaded.
//! Emits RefundsIssued event for external systems to handle refunds.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery},
    utils::{
        account::{read_account_data, write_account_data},
        pda::assert_pda_owned,
        validation::{compute_service_fee, require_key_match, require_writable},
    },
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::Sysvar,
};

/// Process the FinalizeNoAttesters instruction
///
/// Finalizes a lottery when no attestations were submitted by the deadline,
/// or when there are attestations but zero reveals were uploaded by the deadline.
/// This emits a RefundsIssued event that external systems can act upon.
///
/// # Accounts Expected
/// 0. `[]` Config account
/// 1. `[writable]` Lottery account
/// 2. `[writable]` Vault account
/// 3. `[writable]` Authority wallet (refund recipient)
pub fn process(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    // Get accounts
    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let vault_ai = next_account_info(account_info_iter)?;
    let authority_ai = next_account_info(account_info_iter)?;

    // Validate accounts
    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(lottery_ai)?;
    require_writable(vault_ai)?;
    require_writable(authority_ai)?;

    // Read account data
    let config: Config = read_account_data(config_ai)?;
    let mut lottery: Lottery = read_account_data(lottery_ai)?;

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
    require_key_match(vault_ai, &lottery.vault)?;
    require_key_match(authority_ai, &config.authority)?;

    // Debug logging
    solana_program::msg!(
        "FinalizeNoAttesters: lottery_id={} participants={} settled={}",
        lottery.id,
        lottery.participants_count,
        lottery.settled
    );

    // Check if lottery can be finalized
    if lottery.settled {
        return Err(Error::LotteryAlreadySettled.into());
    }

    // Check if upload (attestation) deadline has passed
    let clock = solana_program::clock::Clock::get()?;
    if lottery.upload_deadline_unix == 0 || clock.unix_timestamp < lottery.upload_deadline_unix {
        return Err(Error::InvalidInstruction.into());
    }

    // Valid finalize scenarios after deadline:
    // 1) No attestations at all
    // 2) Some attestations but zero reveals uploaded
    let no_attestations = lottery.attested_count == 0;
    let no_reveals_uploaded = lottery.provider_uploaded_count == 0;
    if !(no_attestations || no_reveals_uploaded) {
        return Err(Error::InvalidInstruction.into());
    }

    // Note: allow zero tickets (noop) to still mark settled

    // Mark lottery as settled
    lottery.settle();
    write_account_data(lottery_ai, "Lottery", &lottery)?;

    // Compute net refund after service fee (off-chain transfer semantics are not performed here;
    // this event communicates the net amount to be refunded by the provider flow)
    let service_fee = compute_service_fee(lottery.total_funds, config.service_charge_bps)?;
    let net_refund = lottery.total_funds.saturating_sub(service_fee);

    // Drain vault to authority so refunds can be processed off-chain.
    let vault_lamports_ref = &mut **vault_ai.try_borrow_mut_lamports()?;
    let authority_lamports_ref = &mut **authority_ai.try_borrow_mut_lamports()?;
    let vault_balance = *vault_lamports_ref;
    if vault_balance > 0 {
        *authority_lamports_ref = authority_lamports_ref.saturating_add(vault_balance);
        *vault_lamports_ref = 0;
    }

    // Emit RefundsIssued event for external systems to handle
    solana_program::msg!(
        "Emitting RefundsIssued: lottery_id={} participants={}",
        lottery.id,
        lottery.participants_count
    );
    let refunds_event = LotteryEvent::RefundsIssued {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        recipient_count: lottery.participants_count,
        total_refunded_lamports: net_refund,
        reason: if lottery.participants_count == 1 {
            "Single participant refunded".to_string()
        } else if no_attestations {
            "No attestations submitted".to_string()
        } else {
            "No reveals uploaded by provider".to_string()
        },
        timestamp: clock.unix_timestamp,
    };
    refunds_event.emit();

    // Emit LotterySettled event to mark the lottery as conclusively settled
    // For refund scenarios (no attesters), we emit with zero payouts/winners
    solana_program::msg!("Emitting LotterySettled: lottery_id={}", lottery.id);
    let settled_event = LotteryEvent::LotterySettled {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        vault: lottery.vault.to_string(),
        winner: String::new(), // No winner in refund scenario
        winning_ticket_index: 0,
        total_tickets: lottery.total_tickets,
        total_funds: lottery.total_funds,
        service_fee,
        winner_payout: 0,              // No winner payout in refund scenario
        selected_number_of_winners: 0, // No winners selected
        authority: lottery.authority.to_string(),
        timestamp: clock.unix_timestamp,
        winners: vec![], // Empty winners list for refund
        per_winner_payout: 0,
    };
    settled_event.emit();

    Ok(())
}
