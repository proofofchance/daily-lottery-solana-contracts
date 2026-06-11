//! # Claim Refund Instruction
//!
//! Allows a participant to claim their ticket payment from a cancelled lottery
//! directly from the program-owned vault.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery, Participant, Vault},
    utils::{
        account::{read_account_data, write_account_data},
        pda::assert_pda_owned,
        validation::{require_key_match, require_signer, require_writable},
    },
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::Sysvar,
};

/// Process the ClaimRefund instruction.
///
/// Accounts expected:
/// 0. `[]` Config account
/// 1. `[writable]` Lottery account
/// 2. `[writable]` Vault account
/// 3. `[writable]` Participant account
/// 4. `[writable, signer]` Participant wallet
pub fn process(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let vault_ai = next_account_info(account_info_iter)?;
    let participant_ai = next_account_info(account_info_iter)?;
    let wallet_ai = next_account_info(account_info_iter)?;

    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(lottery_ai)?;
    require_writable(vault_ai)?;
    require_writable(participant_ai)?;
    require_writable(wallet_ai)?;
    require_signer(wallet_ai)?;

    let config: Config = read_account_data(config_ai)?;
    let lottery: Lottery = read_account_data(lottery_ai)?;
    let _vault: Vault = read_account_data(vault_ai)?;
    let mut participant: Participant = read_account_data(participant_ai)?;

    assert_pda_owned(
        program_id,
        lottery_ai,
        &[
            b"lottery",
            config_ai.key.as_ref(),
            &lottery.id.to_le_bytes(),
        ],
    )?;
    if lottery.config != *config_ai.key {
        return Err(Error::InvalidAccountData.into());
    }

    assert_pda_owned(program_id, vault_ai, &[b"vault", lottery_ai.key.as_ref()])?;
    require_key_match(vault_ai, &lottery.vault)?;

    assert_pda_owned(
        program_id,
        participant_ai,
        &[
            b"participant",
            lottery_ai.key.as_ref(),
            wallet_ai.key.as_ref(),
        ],
    )?;
    if participant.lottery != *lottery_ai.key || participant.wallet != *wallet_ai.key {
        return Err(Error::InvalidAccountData.into());
    }

    if !lottery.settled || lottery.winners_count != 0 || lottery.total_payout != 0 {
        return Err(Error::RefundUnavailable.into());
    }
    if participant.tickets_bought == 0 {
        return Err(Error::RefundUnavailable.into());
    }
    if participant.refund_claimed() {
        return Err(Error::RefundAlreadyClaimed.into());
    }

    let refund_amount = config
        .ticket_price_lamports
        .checked_mul(participant.tickets_bought)
        .ok_or(Error::MathOverflow)?;

    {
        let vault_lamports_ref = &mut **vault_ai.try_borrow_mut_lamports()?;
        let wallet_lamports_ref = &mut **wallet_ai.try_borrow_mut_lamports()?;
        if *vault_lamports_ref < refund_amount {
            return Err(Error::InsufficientFunds.into());
        }

        *vault_lamports_ref = vault_lamports_ref.saturating_sub(refund_amount);
        *wallet_lamports_ref = wallet_lamports_ref.saturating_add(refund_amount);
    }

    participant.mark_refund_claimed();
    write_account_data(participant_ai, "Participant", &participant)?;

    let timestamp = Clock::get()?.unix_timestamp;
    LotteryEvent::RefundClaimed {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        participant: participant_ai.key.to_string(),
        wallet: wallet_ai.key.to_string(),
        amount: refund_amount,
        tickets_refunded: participant.tickets_bought,
        timestamp,
    }
    .emit();

    Ok(())
}
