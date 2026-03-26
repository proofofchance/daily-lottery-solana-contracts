//! # Attest Uploaded Instruction
//!
//! Allows participants to attest that they have uploaded their reveal off-chain.

use crate::{
    error::Error,
    events::LotteryEvent,
    state::{Config, Lottery, Participant},
    utils::{
        account::{read_account_data, write_account_data},
        pda::assert_pda_owned,
        validation::{require_signer, require_writable},
    },
};
use solana_program::ed25519_program;
use solana_program::sysvar::instructions as sysvar_instructions;
use solana_program::sysvar::instructions::get_instruction_relative;
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    pubkey::Pubkey,
    sysvar::Sysvar,
};

const ATTESTATION_MESSAGE_DOMAIN_V2: &[u8] = &[
    0x49, 0x4b, 0x49, 0x47, 0x41, 0x49, 0x5f, 0x41, 0x54, 0x54, 0x45, 0x53, 0x54, 0x5f, 0x56,
    0x32,
];

/// Process the AttestUploaded instruction
///
/// Allows participants to attest that they have uploaded their reveal off-chain.
/// This is part of the anti-censorship mechanism.
///
/// # Accounts Expected
/// 0. `[]` Config account
/// 1. `[]` Lottery account
/// 2. `[writable]` Participant account
/// 3. `[signer]` Participant wallet
pub fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    voted_number_of_winners: u64,
) -> ProgramResult {
    solana_program::msg!(
        "ATTEST_ENTER v2 vote={} accs={}",
        voted_number_of_winners,
        accounts.len()
    );

    // Early explicit account length guard
    if accounts.len() < 5 {
        return Err(Error::InvalidInstruction.into());
    }
    let account_info_iter = &mut accounts.iter();

    // Get accounts
    let config_ai = next_account_info(account_info_iter)?;
    let lottery_ai = next_account_info(account_info_iter)?;
    let participant_ai = next_account_info(account_info_iter)?;
    let wallet_ai = next_account_info(account_info_iter)?;
    let ix_sysvar_ai = next_account_info(account_info_iter)?;

    solana_program::msg!(
        "ATTEST_KEYS cfg={} lot={} part={} wal={} ix={}",
        config_ai.key,
        lottery_ai.key,
        participant_ai.key,
        wallet_ai.key,
        ix_sysvar_ai.key
    );

    // Validate accounts
    assert_pda_owned(program_id, config_ai, &[b"config"])?;
    require_writable(lottery_ai)?;
    require_writable(participant_ai)?;
    require_signer(wallet_ai)?;

    // Validate instructions sysvar
    if ix_sysvar_ai.key != &sysvar_instructions::id() {
        return Err(Error::InvalidInstruction.into());
    }

    // Read account data
    let config: Config = read_account_data(config_ai)?;
    let mut lottery: Lottery = read_account_data(lottery_ai)?;
    let mut participant: Participant = read_account_data(participant_ai)?;

    // Re-derive and assert lottery PDA using loaded lottery.id
    assert_pda_owned(
        program_id,
        lottery_ai,
        &[
            b"lottery",
            config_ai.key.as_ref(),
            &lottery.id.to_le_bytes(),
        ],
    )?;

    // Re-derive and assert participant PDA using loaded wallet key
    assert_pda_owned(
        program_id,
        participant_ai,
        &[
            b"participant",
            lottery_ai.key.as_ref(),
            wallet_ai.key.as_ref(),
        ],
    )?;

    // Verify participant wallet matches signer
    if participant.wallet != *wallet_ai.key {
        return Err(Error::Unauthorized.into());
    }

    // Verify participant has tickets
    if !participant.has_tickets() {
        return Err(Error::InvalidInstruction.into());
    }

    // Check if we're in upload phase (attestation occurs during upload phase)
    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;

    if lottery.phase(current_time) != "upload" {
        return Err(Error::OutsideTimeWindow.into());
    }

    // Validate vote range. Max winners <= participants_count - 1, min 1.
    if voted_number_of_winners == 0 {
        return Err(Error::InvalidInstruction.into());
    }
    if lottery.participants_count <= 1 {
        // In single-participant scenarios, only a vote of 1 is meaningful.
        if voted_number_of_winners != 1 {
            return Err(Error::InvalidInstruction.into());
        }
    }
    if voted_number_of_winners > lottery.participants_count.saturating_sub(1) {
        return Err(Error::InvalidInstruction.into());
    }

    // Check if already attested
    if participant.attested_uploaded {
        return Err(Error::AttestationAlreadySubmitted.into());
    }

    // Verify presence of a valid ed25519 provider signature instruction preceding this ix.
    // Expected message = domain bytes || lottery || wallet || proof_hash || voted_winners (LE u64)
    let mut expected_message: Vec<u8> = ATTESTATION_MESSAGE_DOMAIN_V2.to_vec();
    expected_message.extend_from_slice(lottery_ai.key.as_ref());
    expected_message.extend_from_slice(wallet_ai.key.as_ref());
    expected_message.extend_from_slice(&participant.proof_of_chance_hash);
    expected_message.extend_from_slice(&voted_number_of_winners.to_le_bytes());

    solana_program::msg!("ATTEST_SCAN strict ed25519 parse start");
    let mut provider_sig_ok = false;

    // Helper to parse a single-signature ed25519 verify instruction created via web3.js.
    // Require inline data (instruction index = 0xFFFF) to prevent spoofed bytes.
    fn parse_ed25519_ix<'a>(data: &'a [u8]) -> Option<(&'a [u8; 32], &'a [u8])> {
        // Must contain at least header for 1 signature: 1 (num) + 1 (pad) + 14 bytes
        if data.len() < 16 {
            return None;
        }
        let num = data[0] as usize;
        if num != 1 {
            return None;
        }
        // First signature descriptor begins at offset 2
        let mut rd = 2usize;
        if data.len() < rd + 14 {
            return None;
        }
        let read_u16 =
            |buf: &[u8], off: usize| -> u16 { u16::from_le_bytes([buf[off], buf[off + 1]]) };
        let sig_off = read_u16(data, rd) as usize;
        rd += 2;
        let sig_ix = read_u16(data, rd);
        rd += 2;
        let pk_off = read_u16(data, rd) as usize;
        rd += 2;
        let pk_ix = read_u16(data, rd);
        rd += 2;
        let msg_off = read_u16(data, rd) as usize;
        rd += 2;
        let msg_sz = read_u16(data, rd) as usize;
        rd += 2;
        let msg_ix = read_u16(data, rd);

        // Require inline offsets (ed25519 program will read from this instruction data).
        if sig_ix != u16::MAX || pk_ix != u16::MAX || msg_ix != u16::MAX {
            return None;
        }

        // Bounds checks
        if sig_off >= data.len() {
            return None;
        }
        if pk_off + 32 > data.len() {
            return None;
        }
        if msg_off + msg_sz > data.len() {
            return None;
        }

        // Extract pubkey and message slices
        let pk_bytes: &[u8; 32] = data.get(pk_off..pk_off + 32)?.try_into().ok()?;
        let msg_bytes: &[u8] = &data[msg_off..msg_off + msg_sz];

        // Basic sanity: signature should be present (64 bytes)
        let _sig_end_ok = sig_off + 64 <= data.len();
        if !_sig_end_ok {
            return None;
        }

        Some((pk_bytes, msg_bytes))
    }

    // Only trust ed25519 instructions that precede this call
    for rel in -10..=-1 {
        if let Ok(ix) = get_instruction_relative(rel, ix_sysvar_ai) {
            if ix.program_id == ed25519_program::id() {
                if let Some((pk_bytes, msg_bytes)) = parse_ed25519_ix(&ix.data) {
                    if pk_bytes == &config.authority.to_bytes()
                        && msg_bytes == expected_message.as_slice()
                    {
                        provider_sig_ok = true;
                        break;
                    }
                }
            }
        }
    }

    if !provider_sig_ok {
        return Err(Error::InvalidAttestation.into());
    }

    // Record vote before locking attestation
    participant.set_vote_number_of_winners(voted_number_of_winners)?;

    // Record attestation
    participant.attest_upload(current_time)?;

    // Update lottery attested count
    lottery.add_attestation()?;

    // If everyone has attested, immediately begin settlement (no deadline phase)
    let mut began_settlement = false;
    if lottery.participants_count > 0 && lottery.attested_count == lottery.participants_count {
        let now2 = Clock::get()?.unix_timestamp;
        if lottery.settlement_start_unix == 0 {
            lottery.settlement_start_unix = now2;
            began_settlement = true;
        }
    }

    // Write updated data
    write_account_data(participant_ai, "Participant", &participant)?;
    write_account_data(lottery_ai, "Lottery", &lottery)?;

    // Emit attestation submitted event FIRST
    let event = LotteryEvent::AttestationSubmitted {
        lottery_id: lottery.id,
        lottery: lottery_ai.key.to_string(),
        participant: participant_ai.key.to_string(),
        wallet: wallet_ai.key.to_string(),
        voted_number_of_winners,
        total_attested: lottery.attested_count,
        timestamp: current_time,
    };
    event.emit();

    // If settlement began, emit an awaiting/settlement-began style event (kept until events are renamed)
    if began_settlement {
        let ts = Clock::get()?.unix_timestamp;
        LotteryEvent::SettlementPhaseBegan {
            lottery_id: lottery.id,
            lottery: lottery_ai.key.to_string(),
            settlement_start_unix: lottery.settlement_start_unix,
            timestamp: ts,
        }
        .emit();
    }

    Ok(())
}
