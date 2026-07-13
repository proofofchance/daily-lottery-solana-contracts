//! # Close Participant Instruction
//!
//! Reclaims a participant PDA's rent after the lottery no longer needs that
//! account for refunds, finalization, or payouts.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery, Participant},
    utils::{
        account::read_account_data,
        pda::assert_pda_owned,
        validation::{require_signer, require_writable},
    },
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::Sysvar,
};

/// Process the CloseParticipant instruction.
pub fn process(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let participant_ai = next_account_info(account_info_iter)?;
    let wallet_ai = next_account_info(account_info_iter)?;

    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(participant_ai)?;
    require_writable(wallet_ai)?;
    require_signer(wallet_ai)?;

    let _config: Config = read_account_data(config_ai)?;
    let lottery: Lottery = read_account_data(lottery_ai)?;
    let participant: Participant = read_account_data(participant_ai)?;

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

    let can_close_refund_path =
        lottery.settled && lottery.winners_count == 0 && participant.refund_claimed();
    let can_close_winners_path = lottery.settlement_complete && lottery.winners_count > 0;
    if !can_close_refund_path && !can_close_winners_path {
        return Err(Error::InvalidLotteryState.into());
    }

    let rent_reclaimed = participant_ai.lamports();
    if rent_reclaimed == 0 {
        return Err(Error::InvalidAccount.into());
    }

    {
        let participant_lamports_ref = &mut **participant_ai.try_borrow_mut_lamports()?;
        let wallet_lamports_ref = &mut **wallet_ai.try_borrow_mut_lamports()?;
        *wallet_lamports_ref = wallet_lamports_ref
            .checked_add(*participant_lamports_ref)
            .ok_or(Error::MathOverflow)?;
        *participant_lamports_ref = 0;
    }
    LotteryEvent::ParticipantClosed {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        participant: participant_ai.key.to_string(),
        wallet: wallet_ai.key.to_string(),
        rent_reclaimed,
        timestamp: Clock::get()?.unix_timestamp,
    }
    .emit();

    Ok(())
}
