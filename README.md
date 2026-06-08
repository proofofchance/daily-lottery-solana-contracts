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
- Multiple lotteries may be active concurrently; operational commands should target
  an explicit lottery ID when settling or paying winners.
- Single-participant lotteries auto-complete upload when the upload/attestation
  phase opens and can settle immediately through refund semantics. Multi-participant
  no-attester or zero-reveal refunds remain gated on the upload deadline.
