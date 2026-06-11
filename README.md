# Daily Lottery Solana Contracts

This folder contains the extracted daily lottery Solana program, its helper
source, and tests.

Common local commands:

- `cargo fmt --all --check`
- `cargo test --workspace`
- `npm test`
- `npm run lint`

Notes:

- Upload and attestation refer to the same participant phase.
- Local and staging builds use the `allow-early-upload` feature so operators can
  start upload without waiting for wall-clock deadlines. Production/mainnet
  builds omit that feature.
- Service charge updates are a local/staging operational feature behind
  `allow-service-charge-update`; production/mainnet builds keep the initialized
  fee immutable.
- New ticket purchases are capped by `MAX_SETTLEMENT_PARTICIPANTS` so winner
  finalization can still fit every participant account in one Solana
  transaction.
- Multiple lotteries may be active concurrently; operational commands should target
  an explicit lottery ID when settling or paying winners.
- Single-participant lotteries auto-complete upload when the upload/attestation
  phase opens and can settle immediately through refund semantics. Multi-participant
  no-attester or zero-reveal refunds remain gated on the upload deadline.
- Refund/cancel rounds leave participant funds in the program vault until each
  participant claims with `ClaimRefund`.
- Participants can bypass provider attestation receipts by submitting
  `AttestReveal`, which verifies their reveal against the original commitment and
  includes it in settlement entropy on-chain.
