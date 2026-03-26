const anchor = require("@coral-xyz/anchor");
const assert = require("assert");
const crypto = require("crypto");
const os = require("os");
const { deserializeUnchecked } = require("borsh");

const DEFAULT_PROVIDER_URL = "http://127.0.0.1:8899";
if (!process.env.ANCHOR_PROVIDER_URL) {
  process.env.ANCHOR_PROVIDER_URL = DEFAULT_PROVIDER_URL;
}
if (!process.env.ANCHOR_WALLET) {
  process.env.ANCHOR_WALLET = `${os.homedir()}/.config/solana/id.json`;
}

const TAGS = {
  initialize: 0,
  createLottery: 2,
  buyTickets: 3,
  beginRevealPhase: 8,
  finalizeNoAttesters: 9,
};

function u16le(n) {
  const buf = Buffer.alloc(2);
  buf.writeUInt16LE(n);
  return buf;
}

function u32le(n) {
  const buf = Buffer.alloc(4);
  buf.writeUInt32LE(n >>> 0);
  return buf;
}

function u64le(n) {
  const buf = Buffer.alloc(8);
  buf.writeBigUInt64LE(BigInt(n));
  return buf;
}

function encodeBuyTickets(proofHash, tickets) {
  const parts = [
    Buffer.from([TAGS.buyTickets]),
    Buffer.from([proofHash ? 1 : 0]),
  ];
  if (proofHash) {
    if (proofHash.length !== 32) {
      throw new Error("proof hash must be 32 bytes");
    }
    parts.push(Buffer.from(proofHash));
  }
  parts.push(u64le(tickets));
  return Buffer.concat(parts);
}

class LotteryAccount {
  constructor(fields = {}) {
    Object.assign(this, fields);
  }
}

const LOTTERY_SCHEMA = new Map([
  [
    LotteryAccount,
    {
      kind: "struct",
      fields: [
        ["id", "u64"],
        ["config", [32]],
        ["authority", [32]],
        ["created_at_unix", "i64"],
        ["buy_start_unix", "i64"],
        ["buy_deadline_unix", "i64"],
        ["upload_start_unix", "i64"],
        ["upload_deadline_unix", "i64"],
        ["settlement_start_unix", "i64"],
        ["status", "u8"],
        ["total_tickets", "u64"],
        ["total_funds", "u64"],
        ["provider_uploaded_count", "u64"],
        ["poc_aggregate_hash", [32]],
        ["uploads_complete", "bool"],
        ["settled", "bool"],
        ["vault", [32]],
        ["vault_bump", "u8"],
        ["attested_count", "u64"],
        ["participants_count", "u64"],
        ["selected_number_of_winners", "u64"],
        ["winners_merkle_root", [32]],
        ["winners_count", "u64"],
        ["total_payout", "u64"],
        ["paid_winners_bitmap", ["u8"]],
        ["settlement_batches_completed", "u32"],
        ["settlement_complete", "bool"],
      ],
    },
  ],
]);

describe("daily-lottery lifecycle (raw tx)", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const programId = new anchor.web3.PublicKey(
    "BknqbA2DPtJgsqQDVPHiTXUds5ve512FxT8cpMh6fLZn"
  );
  const payer = provider.wallet.payer ?? provider.wallet;
  const [configPda] = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    programId
  );
  const lotteryId = 1n;
  const lotterySeeds = [
    Buffer.from("lottery"),
    configPda.toBuffer(),
    u64le(lotteryId),
  ];
  const [lotteryPda] = anchor.web3.PublicKey.findProgramAddressSync(
    lotterySeeds,
    programId
  );
  const [vaultPda] = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), lotteryPda.toBuffer()],
    programId
  );
  const [participantPda] = anchor.web3.PublicKey.findProgramAddressSync(
    [
      Buffer.from("participant"),
      lotteryPda.toBuffer(),
      payer.publicKey.toBuffer(),
    ],
    programId
  );
  const proofHash = crypto
    .createHash("sha256")
    .update("abracadabra")
    .digest();

  async function sendAndLog(tx) {
    return provider.sendAndConfirm(tx).catch((err) => {
      console.error("transaction failed", err);
      throw err;
    });
  }

  async function readLottery() {
    const info = await provider.connection.getAccountInfo(lotteryPda);
    assert.ok(info, "lottery account not found");
    return deserializeUnchecked(LOTTERY_SCHEMA, LotteryAccount, info.data);
  }

  it("initialize config (idempotent)", async () => {
    const data = Buffer.concat([
      Buffer.from([TAGS.initialize]),
      u64le(1_000_000),
      u16le(500),
      u32le(32),
    ]);
    const keys = [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      { pubkey: configPda, isSigner: false, isWritable: true },
      {
        pubkey: anchor.web3.SystemProgram.programId,
        isSigner: false,
        isWritable: false,
      },
    ];
    const ix = new anchor.web3.TransactionInstruction({
      keys,
      programId,
      data,
    });
    const sig = await sendAndLog(
      new anchor.web3.Transaction().add(ix)
    );
    assert.ok(sig.length > 0);
  });

  it("create lottery (id=1)", async () => {
    const data = Buffer.from([TAGS.createLottery]);
    const keys = [
      { pubkey: configPda, isSigner: false, isWritable: true },
      { pubkey: lotteryPda, isSigner: false, isWritable: true },
      { pubkey: vaultPda, isSigner: false, isWritable: true },
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      {
        pubkey: anchor.web3.SystemProgram.programId,
        isSigner: false,
        isWritable: false,
      },
    ];
    const ix = new anchor.web3.TransactionInstruction({
      keys,
      programId,
      data,
    });
    const sig = await sendAndLog(
      new anchor.web3.Transaction().add(ix)
    );
    assert.ok(sig.length > 0);

    const lottery = await readLottery();
    assert.strictEqual(lottery.id.toNumber(), 1, "lottery id mismatch");
  });

  it("buy tickets (single participant)", async () => {
    const data = encodeBuyTickets(proofHash, 3);
    const keys = [
      { pubkey: configPda, isSigner: false, isWritable: false },
      { pubkey: lotteryPda, isSigner: false, isWritable: true },
      { pubkey: vaultPda, isSigner: false, isWritable: true },
      { pubkey: participantPda, isSigner: false, isWritable: true },
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      {
        pubkey: anchor.web3.SystemProgram.programId,
        isSigner: false,
        isWritable: false,
      },
    ];
    const ix = new anchor.web3.TransactionInstruction({
      keys,
      programId,
      data,
    });
    const sig = await sendAndLog(
      new anchor.web3.Transaction().add(ix)
    );
    assert.ok(sig.length > 0);

    const participantInfo = await provider.connection.getAccountInfo(
      participantPda
    );
    assert.ok(participantInfo, "participant account missing");

    const lottery = await readLottery();
    assert.strictEqual(
      lottery.participants_count.toNumber(),
      1,
      "participants count should be 1"
    );
    assert.strictEqual(
      lottery.total_tickets.toNumber(),
      3,
      "total tickets mismatch"
    );
  });

  it("force upload phase for single participant", async () => {
    const data = Buffer.from([TAGS.beginRevealPhase]);
    const keys = [
      { pubkey: configPda, isSigner: false, isWritable: false },
      { pubkey: lotteryPda, isSigner: false, isWritable: true },
      { pubkey: payer.publicKey, isSigner: true, isWritable: false },
    ];
    const ix = new anchor.web3.TransactionInstruction({
      keys,
      programId,
      data,
    });
    const sig = await sendAndLog(
      new anchor.web3.Transaction().add(ix)
    );
    assert.ok(sig.length > 0);

    const lottery = await readLottery();
    assert.strictEqual(
      lottery.uploads_complete,
      true,
      "uploads should be marked complete for single participant"
    );
    assert.strictEqual(
      lottery.selected_number_of_winners.toNumber(),
      1,
      "selected winners should default to 1"
    );
  });

  it("finalize via no attesters (refund path)", async () => {
    const data = Buffer.from([TAGS.finalizeNoAttesters]);
    const keys = [
      { pubkey: configPda, isSigner: false, isWritable: false },
      { pubkey: lotteryPda, isSigner: false, isWritable: true },
      { pubkey: payer.publicKey, isSigner: true, isWritable: false },
    ];
    const ix = new anchor.web3.TransactionInstruction({
      keys,
      programId,
      data,
    });
    const sig = await sendAndLog(
      new anchor.web3.Transaction().add(ix)
    );
    assert.ok(sig.length > 0);

    const lottery = await readLottery();
    assert.strictEqual(lottery.settled, true, "lottery not settled");
    assert.strictEqual(
      lottery.attested_count.toNumber(),
      0,
      "attested count should remain zero"
    );
    assert.strictEqual(
      lottery.participants_count.toNumber(),
      1,
      "participants count changed unexpectedly"
    );
  });
});
