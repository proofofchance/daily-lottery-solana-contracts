# Daily Lottery Solana Contracts

This folder contains the extracted daily lottery Solana program, its helper
source, and tests.

Operational files live at the repo root:

- requirements: `DAILY_LOTTERY_SOLANA_CONTRACTS_REQUIREMENTS.md`
- Make targets: `daily-lottery-solana-contracts.mk`
- scripts: `solana-scripts/daily-lottery/`
- secrets and env: root `.env` plus root `.secrets/`

Use the root Make targets for build, deploy, and workflow commands. The main
ones are:

- `make daily-lottery-contracts.build`
- `make daily-lottery-contracts.test`
- `make daily-lottery-contracts.deploy ENV=devnet`
- `make create-lottery`
- `make begin-upload`
- `make reveal-lottery id=<LOTTERY_ID>`
- `make payout-winners id=<LOTTERY_ID>`

Notes:

- Upload and attestation refer to the same participant phase.
- `LotteryCreated` emits both buy and upload window timestamps. Keep these
  fields in sync with the account defaults in `Lottery::calculate_windows` so
  indexers can reconstruct the phase without fetching the account.
- Reveal and payout scripts depend on the Ark PG backend when used through the
  root workflow.
- The reveal and payout walkthrough remains in
  `solana-scripts/daily-lottery/settlement-workflow.md`.
