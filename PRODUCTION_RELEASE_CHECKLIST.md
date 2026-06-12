# Daily Lottery Production Release Checklist

Use this checklist for every mainnet daily lottery deploy or upgrade. The public
repository is the source-verification input; do not rely on private workspace
state for production builds.

## Required Evidence

- Public repo commit SHA that contains the exact build inputs.
- Production SBF artifact built without `allow-early-upload` or
  `allow-service-charge-update`.
- Verified build log from `solana-verify build`.
- Deployed program id, programdata address, deployment slot, and deploy
  signature.
- Verified source result from `solana-verify verify-from-repo`.
- On-chain upgrade authority pubkey after deployment.
- The Ark deployment manifest and runtime env snapshot refreshed from the
  deployed program metadata.
- Public protocol upgrade notice containing commit SHA, program id, deployment
  slot, verification result, and summary of instruction/account/event changes.

## Pre-Deploy

1. Confirm the private-to-public sync is clean from the monorepo root:

   ```bash
   make daily-lottery-contracts.check-sync-from-private
   ```

2. Confirm public build inputs are committed:

   ```bash
   git status --short -- Cargo.toml Cargo.lock rust-toolchain.toml programs vendor
   ```

   The command must print nothing.

3. Run production and test-feature validation from this public repo:

   ```bash
   cargo fmt --all --check
   cargo build-sbf --manifest-path programs/daily_lottery/Cargo.toml
   cargo build-sbf --manifest-path programs/daily_lottery/Cargo.toml -- --features allow-early-upload,allow-service-charge-update -p daily_lottery
   cargo test -p daily_lottery --features allow-early-upload,allow-service-charge-update
   ```

4. Run the verified-build prerequisite check from the monorepo root:

   ```bash
   make daily-lottery-contracts.check-verified-deploy-prereqs ENV=mainnet
   ```

## Deploy

Prefer the verified deploy target:

```bash
make daily-lottery-contracts.deploy-verified \
  ENV=mainnet \
  POC_VERIFY_REPO_URL=https://github.com/proofofchance/daily-lottery-solana-contracts \
  POC_VERIFY_COMMIT_HASH=<public-commit-sha>
```

The mainnet target must refuse builds that include testing/admin features and
must verify the upgrade authority immediately after deployment.

## Post-Deploy

1. Refresh The Ark program deployment metadata and runtime snapshot:

   ```bash
   ./scripts/poc-env.sh production ark -- make -e -C the-ark-pg deployment-manifest-refresh
   ./scripts/poc-env.sh production ark -- make -e -C the-ark-pg deployment-runtime-env-refresh
   ./scripts/poc-verify-enabled-deployment-manifest.sh production
   ```

2. Publish or update the protocol upgrade notice through The Ark admin API before
   users rely on the new program version. The final notice must include the
   public commit SHA, deployment slot, deploy signature, programdata address,
   source verification result, and user-visible protocol changes.

3. Verify the deployed backend after The Ark is updated. Use the production
   health endpoint and confirm indexer status reports the refreshed deployment
   slot for `daily-lottery`.

## Contract Invariants

- Ticket purchases have no participant-count cap.
- Winner finalization is chunked through `FinalizationLedger`.
- Winner-count votes are bounded by the effective configured winner capacity.
- Selection chunks only accept participants already included during aggregation.
- Refund/cancel paths leave participant funds in the program vault until
  `ClaimRefund`.
- Each lottery snapshots `ticket_price_lamports` and `service_charge_bps` at
  creation; buy, refund, finalization, and payout settlement math must use the
  lottery snapshots, not mutable config.
- Production builds omit early-upload and service-charge-update features.
