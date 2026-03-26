use std::{
    env,
    path::{Path, PathBuf},
};

use litesvm::{
    types::{FailedTransactionMetadata, TransactionResult},
    LiteSVM,
};
use solana_program::{clock::Clock, pubkey::Pubkey};
use solana_account::Account;
use solana_instruction::{error::InstructionError, Instruction};
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

const DEFAULT_AIRDROP_LAMPORTS: u64 = 10_000_000_000;
#[allow(dead_code)]
const APPROX_SECONDS_PER_SLOT_DIVISOR: u64 = 2;

pub struct TestContext {
    pub svm: LiteSVM,
    pub payer: Keypair,
}

#[allow(dead_code)]
impl TestContext {
    pub fn new(program_id: Pubkey, extra_funded_accounts: &[&Keypair]) -> Self {
        let mut svm = LiteSVM::new();
        svm.add_program_from_file(program_id, program_binary_path("daily_lottery"))
            .expect("failed to load daily_lottery SBF artifact");

        let mut clock = svm.get_sysvar::<Clock>();
        clock.slot = 1;
        clock.unix_timestamp = 1;
        clock.epoch_start_timestamp = 1;
        svm.set_sysvar::<Clock>(&clock);

        let payer = Keypair::new();
        let mut funded_accounts = Vec::with_capacity(extra_funded_accounts.len() + 1);
        funded_accounts.push(&payer);
        funded_accounts.extend_from_slice(extra_funded_accounts);

        for keypair in funded_accounts {
            svm.airdrop(&keypair.pubkey(), DEFAULT_AIRDROP_LAMPORTS)
                .expect("airdrop should succeed");
        }

        Self { svm, payer }
    }

    pub fn get_account(&self, address: Pubkey) -> Option<Account> {
        self.svm.get_account(&address)
    }

    pub fn set_account(&mut self, address: Pubkey, account: Account) {
        self.svm
            .set_account(address, account)
            .expect("set_account should succeed");
    }

    pub fn send_tx(
        &mut self,
        instructions: Vec<Instruction>,
        signers: &[&Keypair],
    ) -> TransactionResult {
        let mut tx = Transaction::new_with_payer(&instructions, Some(&self.payer.pubkey()));
        let mut all_signers = vec![&self.payer];
        all_signers.extend_from_slice(signers);
        tx.sign(&all_signers, self.svm.latest_blockhash());
        self.svm.send_transaction(tx)
    }

    pub fn warp_to_slot(&mut self, slot: u64) {
        let previous_clock = self.svm.get_sysvar::<Clock>();
        self.svm.warp_to_slot(slot);
        let mut clock = self.svm.get_sysvar::<Clock>();
        let slots_advanced = slot.saturating_sub(previous_clock.slot);
        clock.slot = slot;
        clock.unix_timestamp = previous_clock
            .unix_timestamp
            .saturating_add((slots_advanced / APPROX_SECONDS_PER_SLOT_DIVISOR) as i64);
        self.svm.set_sysvar::<Clock>(&clock);
    }

    pub fn get_clock(&self) -> Clock {
        self.svm.get_sysvar::<Clock>()
    }

    pub fn set_clock(&mut self, clock: &Clock) {
        self.svm.set_sysvar::<Clock>(clock);
    }
}

#[allow(dead_code)]
pub fn assert_custom_error(err: FailedTransactionMetadata, code: u32) {
    match err.err {
        TransactionError::InstructionError(_, InstructionError::Custom(actual)) => {
            assert_eq!(actual, code)
        }
        other => panic!("expected custom program error, got {other:?}"),
    }
}

fn program_binary_path(program_name: &str) -> PathBuf {
    let file_name = format!("{program_name}.so");
    for base in candidate_program_dirs() {
        let path = base.join(&file_name);
        if path.exists() {
            return path;
        }
    }

    panic!(
        "missing SBF artifact `{file_name}`; run `cargo test-sbf -p daily_lottery` so LiteSVM can load the compiled program"
    );
}

fn candidate_program_dirs() -> Vec<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut dirs = Vec::new();

    for var in ["BPF_OUT_DIR", "SBF_OUT_DIR"] {
        if let Some(value) = env::var_os(var) {
            dirs.push(PathBuf::from(value));
        }
    }

    dirs.push(manifest_dir.join("target/deploy"));
    dirs.push(manifest_dir.join("../../target/deploy"));
    dirs.push(manifest_dir.join("../target/deploy"));
    dirs
        .into_iter()
        .map(normalize_path)
        .collect::<Vec<_>>()
}

fn normalize_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
    }
}
