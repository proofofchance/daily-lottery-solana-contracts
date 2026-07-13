//! # Close Refund Vault Instruction
//!
//! Reclaims vault rent after a cancelled/refund lottery has paid every
//! participant refund.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery, Vault},
    utils::{
        account::read_account_data,
        pda::assert_pda_owned,
        validation::{require_key_match, require_writable},
    },
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::Sysvar,
};

/// Process the CloseRefundVault instruction.
///
/// Accounts expected:
/// 0. `[]` Config account
/// 1. `[]` Lottery account
/// 2. `[writable]` Vault account
/// 3. `[writable]` Authority wallet
pub fn process(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let vault_ai = next_account_info(account_info_iter)?;
    let authority_ai = next_account_info(account_info_iter)?;

    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(vault_ai)?;
    require_writable(authority_ai)?;

    let config: Config = read_account_data(config_ai)?;
    let lottery: Lottery = read_account_data(lottery_ai)?;
    let _vault: Vault = read_account_data(vault_ai)?;

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
    require_key_match(authority_ai, &config.authority)?;

    if !lottery.settled || lottery.winners_count != 0 || lottery.total_payout != 0 {
        return Err(Error::RefundUnavailable.into());
    }
    if u64::from(lottery.settlement_batches_completed) < lottery.participants_count {
        return Err(Error::RefundUnavailable.into());
    }

    let rent_reclaimed = vault_ai.lamports();
    if rent_reclaimed == 0 {
        return Err(Error::RefundUnavailable.into());
    }

    {
        let vault_lamports_ref = &mut **vault_ai.try_borrow_mut_lamports()?;
        let authority_lamports_ref = &mut **authority_ai.try_borrow_mut_lamports()?;
        *authority_lamports_ref = authority_lamports_ref.saturating_add(*vault_lamports_ref);
        *vault_lamports_ref = 0;
    }

    LotteryEvent::RefundVaultClosed {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        vault: vault_ai.key.to_string(),
        authority: authority_ai.key.to_string(),
        refunds_claimed_count: lottery.settlement_batches_completed,
        vault_rent_reclaimed: rent_reclaimed,
        timestamp: Clock::get()?.unix_timestamp,
    }
    .emit();

    Ok(())
}
