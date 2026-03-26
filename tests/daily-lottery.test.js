const anchor = require("@coral-xyz/anchor");
const assert = require("assert");
const os = require("os");

const DEFAULT_PROVIDER_URL = "http://127.0.0.1:8899";
if (!process.env.ANCHOR_PROVIDER_URL) {
  process.env.ANCHOR_PROVIDER_URL = DEFAULT_PROVIDER_URL;
}
if (!process.env.ANCHOR_WALLET) {
  process.env.ANCHOR_WALLET = `${os.homedir()}/.config/solana/id.json`;
}

const TAGS = {
  initialize: 0,
};

function u16le(n) {
  const buf = Buffer.alloc(2);
  buf.writeUInt16LE(n);
  return buf;
}

function u32le(n) {
  const buf = Buffer.alloc(4);
  buf.writeUInt32LE(n);
  return buf;
}

function u64le(n) {
  const buf = Buffer.alloc(8);
  buf.writeBigUInt64LE(BigInt(n));
  return buf;
}

describe("daily-lottery (raw tx)", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const programId = new anchor.web3.PublicKey(
    "BknqbA2DPtJgsqQDVPHiTXUds5ve512FxT8cpMh6fLZn"
  );
  const [configPda] = anchor.web3.PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
    programId
  );

  it("initialize config", async () => {
    const payer = provider.wallet.payer ?? provider.wallet; // NodeWallet exposes payer

    const data = Buffer.concat([
      Buffer.from([TAGS.initialize]),
      u64le(1_000_000),
      u16le(500),
      u32le(32), // max winners cap
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

    const sig = await provider
      .sendAndConfirm(new anchor.web3.Transaction().add(ix))
      .catch((err) => {
        console.error("initialize config failed", err);
        throw err;
      });

    assert.ok(typeof sig === "string" && sig.length > 0);
    const info = await provider.connection.getAccountInfo(configPda);
    assert.ok(
      info && info.data && info.data.length > 0,
      "config account missing data"
    );
    assert.ok(
      info.owner.equals(programId),
      "config account not owned by program"
    );
  });
});
