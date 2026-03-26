#[cfg(test)]
mod tests {
    use super::*;
    use solana_program_test::*;
    use solana_sdk::{signature::Keypair, signer::Signer, transaction::Transaction};
    use solana_system_interface::program as system_program;

    #[tokio::test]
    async fn test_initialize_and_buy() {
        let program_id = Pubkey::new_unique();
        let mut pt = ProgramTest::new(
            "daily_lottery",
            program_id,
            processor!(process_instruction),
        );

        let (config_pda, _) = Pubkey::find_program_address(&[b"config"], &program_id);
        pt.add_account(
            config_pda,
            solana_program_test::ProgramTest::rent()
                .unwrap()
                .minimum_balance(CONFIG_SIZE)
                .into(),
        );

        let (mut banks_client, payer, recent_blockhash) = pt.start().await;
        let authority = Keypair::new();

        // Initialize
        let init_ix = solana_sdk::instruction::Instruction {
            program_id,
            accounts: vec![
                solana_sdk::instruction::AccountMeta::new(authority.pubkey(), true),
                solana_sdk::instruction::AccountMeta::new(config_pda, false),
                solana_sdk::instruction::AccountMeta::new_readonly(system_program::id(), false),
            ],
            data: Instruction::Initialize {
                ticket_price_lamports: 1_000_000,
                service_charge_bps: 500,
                max_winners_cap: 32,
            }
            .try_to_vec()
            .unwrap(),
        };
        let mut tx = Transaction::new_with_payer(&[init_ix], Some(&payer.pubkey()));
        tx.sign(&[&payer, &authority], recent_blockhash);
        banks_client.process_transaction(tx).await.unwrap();
    }
}
