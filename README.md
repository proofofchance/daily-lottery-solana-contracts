# Daily Lottery Solana Contracts

This folder contains the extracted daily lottery Solana program, its helper
source, and tests.

Common local commands:

- `cargo fmt --all --check`
- `cargo test --workspace --features "allow-early-upload allow-service-charge-update"`
- `npm test`
- `npm run lint`

Production release checklist:

- [PRODUCTION_RELEASE_CHECKLIST.md](PRODUCTION_RELEASE_CHECKLIST.md)

Notes:

- Upload and attestation refer to the same participant phase.
- Local and staging builds use the `allow-early-upload` feature so operators can
  start upload without waiting for wall-clock deadlines. Production/mainnet
  builds omit that feature.
- Service charge updates are a local/staging operational feature behind
  `allow-service-charge-update`; production/mainnet builds keep the initialized
  fee immutable. Each lottery snapshots `ticket_price_lamports` and
  `service_charge_bps` at creation, so a later config change cannot alter
  already-created buy, refund, or payout math.
- Winner finalization is chunked through a `FinalizationLedger` PDA. Ticket
  purchases are not capped by participant count; sorted participant chunks are
  submitted until aggregation and weighted winner selection complete.
- Lottery accounts include an explicit layout version and reserved bytes so
  future controlled upgrades can add fields without immediately changing the
  serialized account size.
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
