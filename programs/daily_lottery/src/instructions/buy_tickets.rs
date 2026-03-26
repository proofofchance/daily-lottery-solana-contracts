//! # Buy Tickets Instruction
//!
//! Allows participants to purchase lottery tickets with proof-of-chance.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery, Participant, Vault},
    utils::{
        account::{read_account_data, write_account_data},
        pda::{assert_pda_owned, derive_participant_pda},
        validation::{require_key_match, require_signer, require_writable, validate_ticket_count},
    },
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    program::{invoke, invoke_signed},
    pubkey::Pubkey,
    rent::Rent,
    sysvar::Sysvar,
};
use solana_system_interface::{instruction as system_instruction, program as system_program};

/// Process the BuyTickets instruction
///
/// Allows users to purchase lottery tickets by providing proof-of-chance hash
/// and payment. Creates or updates participant account.
///
/// # Accounts Expected
/// 0. `[]` Config account
/// 1. `[writable]` Lottery account
/// 2. `[writable]` Vault account (receives payment)
/// 3. `[writable]` Participant account (PDA, may be created)
/// 4. `[signer, writable]` Buyer (pays for tickets and account creation if needed)
/// 5. `[]` System program
pub fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    proof_of_chance_hash: Option<[u8; 32]>,
    number_of_tickets: u64,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    // Get accounts
    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let vault_ai = next_account_info(account_info_iter)?;
    let participant_ai = next_account_info(account_info_iter)?;
    let buyer_ai = next_account_info(account_info_iter)?;
    let system_program_ai = next_account_info(account_info_iter)?;

    // Validate input
    // Subsequent purchases should NOT require a proof. Only first purchase must provide it.
    let provided_proof = proof_of_chance_hash; // Option<[u8; 32]>
    validate_ticket_count(number_of_tickets)?;

    // Validate config PDA and signer
    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_signer(buyer_ai)?;
    require_writable(lottery_ai)?;
    require_writable(vault_ai)?;
    require_writable(participant_ai)?;
    require_writable(buyer_ai)?;
    require_key_match(system_program_ai, &system_program::id())?;

    // Read account data
    let config: Config = read_account_data(config_ai)?;
    let mut lottery: Lottery = read_account_data(lottery_ai)?;
    let _vault: Vault = read_account_data(vault_ai)?;

    // Validate lottery PDA + linkage to config
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

    // Validate vault PDA + linkage to lottery
    assert_pda_owned(program_id, vault_ai, &[b"vault", lottery_ai.key.as_ref()])?;
    require_key_match(vault_ai, &lottery.vault)?;

    // Verify we are within the buy window and lottery is active
    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;
    let upload_started = lottery.upload_start_unix > 0 && current_time >= lottery.upload_start_unix;
    if upload_started {
        return Err(Error::OutsideTimeWindow.into());
    }
    if !lottery.is_active() || !lottery.is_in_buy_window(current_time) {
        return Err(Error::OutsideTimeWindow.into());
    }

    // Derive participant PDA
    let (participant_pubkey, participant_bump) =
        derive_participant_pda(program_id, lottery_ai.key, buyer_ai.key);
    if participant_ai.key != &participant_pubkey {
        return Err(Error::InvalidSeeds.into());
    }

    // Calculate payment
    let total_cost = config
        .ticket_price_lamports
        .checked_mul(number_of_tickets)
        .ok_or(Error::MathOverflow)?;

    // Check if participant account exists
    let participant_exists = participant_ai.data_len() > 0;
    let mut participant = if participant_exists {
        // Load existing participant
        let existing: Participant = read_account_data(participant_ai)?;
        if existing.lottery != *lottery_ai.key || existing.wallet != *buyer_ai.key {
            return Err(Error::InvalidAccountData.into());
        }

        // If a proof is provided on subsequent purchases, it must match; otherwise ignore
        if let Some(hash) = provided_proof {
            if existing.proof_of_chance_hash != hash {
                return Err(Error::ProofHashMismatch.into());
            }
        }

        existing
    } else {
        // Create new participant account
        let rent = Rent::get()?;
        let participant_space = crate::state::sizes::PARTICIPANT_SIZE;
        let participant_rent = rent.minimum_balance(participant_space);

        let participant_seeds = [
            b"participant",
            lottery_ai.key.as_ref(),
            buyer_ai.key.as_ref(),
            &[participant_bump],
        ];

        let create_participant_ix = system_instruction::create_account(
            buyer_ai.key,
            participant_ai.key,
            participant_rent,
            participant_space as u64,
            program_id,
        );

        invoke_signed(
            &create_participant_ix,
            &[
                buyer_ai.clone(),
                participant_ai.clone(),
                system_program_ai.clone(),
            ],
            &[&participant_seeds],
        )?;

        // Initialize new participant (first purchase MUST include a proof)
        let proof_hash = provided_proof.ok_or(Error::MissingProofOfChance)?;
        let participant = Participant::new(
            *lottery_ai.key,
            *buyer_ai.key,
            proof_hash,
            0, // Will be updated by add_tickets call below
        );

        // New participant, increment participants_count
        lottery.add_participant()?;

        participant
    };

    // Add tickets to participant
    participant.add_tickets(number_of_tickets)?;

    // Transfer payment from buyer to vault
    let transfer_ix = system_instruction::transfer(buyer_ai.key, vault_ai.key, total_cost);

    invoke(&transfer_ix, &[buyer_ai.clone(), vault_ai.clone()])?;

    // Update lottery totals
    lottery.add_tickets(number_of_tickets, total_cost)?;

    // Write updated data
    write_account_data(participant_ai, "Participant", &participant)?;
    write_account_data(lottery_ai, "Lottery", &lottery)?;

    // Emit tickets purchased event
    let event = LotteryEvent::TicketsPurchased {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        participant: participant_ai.key.to_string(),
        buyer: buyer_ai.key.to_string(),
        tickets_bought: number_of_tickets,
        total_tickets_for_participant: participant.tickets_bought,
        total_tickets_for_lottery: lottery.total_tickets,
        amount_paid: total_cost,
        total_funds: lottery.total_funds,
        proof_of_chance_hash: provided_proof,
        timestamp: current_time,
    };
    event.emit();

    Ok(())
}
