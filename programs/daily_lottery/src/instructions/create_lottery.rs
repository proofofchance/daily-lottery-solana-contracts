//! # Create Lottery Instruction
//!
//! Creates a new daily lottery instance with its associated vault.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery, Vault},
    utils::{
        account::{read_account_data, write_account_data},
        pda::{assert_pda_owned, derive_lottery_pda, derive_vault_pda},
        validation::{require_key_match, require_signer, require_writable},
    },
};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    program::invoke_signed,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::Sysvar,
};
use solana_system_interface::{instruction as system_instruction, program as system_program};

/// Process the CreateLottery instruction
///
/// Creates a new lottery instance with associated vault account.
/// Only the authority can create new lotteries. Multiple lotteries may be active concurrently.
///
/// # Accounts Expected
/// 0. `[]` Config account
/// 1. `[writable]` Lottery account (PDA, will be created)
/// 2. `[writable]` Vault account (PDA, will be created)
/// 3. `[signer, writable]` Authority (pays for account creation)
/// 4. `[]` System program
pub fn process(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    // Get accounts
    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let vault_ai = next_account_info(account_info_iter)?;
    let authority_ai = next_account_info(account_info_iter)?;
    let system_program_ai = next_account_info(account_info_iter)?;

    // Validate config PDA
    solana_program::msg!("CreateLottery: validating config PDA");
    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_signer(authority_ai)?;
    require_writable(lottery_ai)?;
    require_writable(vault_ai)?;
    require_writable(authority_ai)?;
    require_key_match(system_program_ai, &system_program::id())?;

    // Read config
    solana_program::msg!(
        "Reading config account, data length: {}",
        config_ai.data.borrow().len()
    );
    solana_program::msg!("Config struct size: {}", std::mem::size_of::<Config>());
    let mut config: Config = read_account_data(config_ai)?;

    // Validate authority
    if authority_ai.key != &config.authority {
        return Err(Error::Unauthorized.into());
    }

    // Note: previously enforced a single active-lottery constraint via `open_active`.
    // This constraint has been removed to allow multiple concurrent lotteries.

    // Get current time
    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;

    // Calculate next lottery ID
    let lottery_id = config.lottery_count + 1;

    // Derive lottery PDA
    let (lottery_pubkey, lottery_bump) = derive_lottery_pda(program_id, config_ai.key, lottery_id);
    solana_program::msg!(
        "CreateLottery: expected lottery {} provided {}",
        lottery_pubkey,
        lottery_ai.key
    );
    if lottery_ai.key != &lottery_pubkey {
        return Err(Error::InvalidSeeds.into());
    }

    // Derive vault PDA
    let (vault_pubkey, vault_bump) = derive_vault_pda(program_id, &lottery_pubkey);
    solana_program::msg!(
        "CreateLottery: expected vault {} provided {}",
        vault_pubkey,
        vault_ai.key
    );
    if vault_ai.key != &vault_pubkey {
        return Err(Error::InvalidSeeds.into());
    }

    // Calculate rent for lottery account (bitmap sized to MAX_WINNERS)
    let rent = Rent::get()?;
    let bitmap_bytes = crate::state::sizes::MAX_WINNERS.div_ceil(8);
    // Base (without vec payload):
    let base = 8
        + 8
        + 32
        + 32
        + 8
        + 8
        + 8
        + 8
        + 8
        + 8
        + 1
        + 8
        + 8
        + 8
        + 32
        + 1
        + 1
        + 32
        + 1
        + 8
        + 8
        + 8
        + 32
        + 8
        + 8
        + 4;
    let lottery_space = base + 4 /*vec len*/ + bitmap_bytes + 4 /*settlement_batches_completed*/ + 1 /*settlement_complete*/;
    let lottery_rent = rent.minimum_balance(lottery_space);

    // Calculate rent for vault account
    let vault_space = crate::state::sizes::VAULT_SIZE;
    let vault_rent = rent.minimum_balance(vault_space);

    // Create lottery account if needed (idempotent)
    if lottery_ai.owner != program_id {
        let lottery_seeds = [
            b"lottery",
            config_ai.key.as_ref(),
            &lottery_id.to_le_bytes(),
            &[lottery_bump],
        ];

        let create_lottery_ix = system_instruction::create_account(
            authority_ai.key,
            lottery_ai.key,
            lottery_rent,
            lottery_space as u64,
            program_id,
        );

        invoke_signed(
            &create_lottery_ix,
            &[
                authority_ai.clone(),
                lottery_ai.clone(),
                system_program_ai.clone(),
            ],
            &[&lottery_seeds],
        )?;
    }

    // Create vault account if needed (idempotent)
    if vault_ai.owner != program_id {
        let vault_seeds = [b"vault", lottery_ai.key.as_ref(), &[vault_bump]];

        let create_vault_ix = system_instruction::create_account(
            authority_ai.key,
            vault_ai.key,
            vault_rent,
            vault_space as u64,
            program_id,
        );

        invoke_signed(
            &create_vault_ix,
            &[
                authority_ai.clone(),
                vault_ai.clone(),
                system_program_ai.clone(),
            ],
            &[&vault_seeds],
        )?;
    }

    // Initialize lottery data (always write, even if account exists but uninitialized)
    let lottery = Lottery::new(
        lottery_id,
        *config_ai.key,
        config.authority,
        current_time,
        *vault_ai.key,
        vault_bump,
    );

    // Initialize vault data
    let vault = Vault::new(*lottery_ai.key, vault_bump);

    // Write account data - ensure accounts are properly initialized
    write_account_data(lottery_ai, "Lottery", &lottery)?;
    write_account_data(vault_ai, "Vault", &vault)?;

    // Update config
    config.lottery_count = lottery_id;

    write_account_data(config_ai, "Config", &config)?;

    // Emit lottery created event
    let event = LotteryEvent::LotteryCreated {
        lottery_id,
        lottery: lottery_ai.key.to_string(),
        config: config_ai.key.to_string(),
        authority: config.authority.to_string(),
        vault: vault_ai.key.to_string(),
        created_at_unix: current_time,
        buy_start_unix: lottery.buy_start_unix,
        buy_deadline_unix: lottery.buy_deadline_unix,
        upload_start_unix: lottery.upload_start_unix,
        upload_deadline_unix: lottery.upload_deadline_unix,
    };
    event.emit();

    Ok(())
}
