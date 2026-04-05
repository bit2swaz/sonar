/**
 * Phase 2.3 — Full TDD Integration Test Suite
 *
 * 17 tests covering all instruction paths of the Sonar ZK-coprocessor program:
 *   - ACCESS CONTROL  (3)
 *   - REQUEST FLOW    (3)
 *   - CALLBACK FLOW   (5)
 *   - REFUND FLOW     (2)
 *   - EDGE CASES      (4)
 *
 * Proof fixture: groth16-solana v0.2.0 built-in demo circuit.
 *   proof_a supplied to Groth16Verifier must already be the *negated* G1 point.
 *   VALID_PROOF below is: [neg(proof_a) | proof_b | proof_c] (256 bytes).
 */

import * as anchor from "@coral-xyz/anchor";
import { AnchorError } from "@coral-xyz/anchor";
import { assert } from "chai";
import { readFileSync } from "fs";
import { join } from "path";
import {
  ComputeBudgetProgram,
  Keypair,
  PublicKey,
  SystemProgram,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";

// ---------------------------------------------------------------------------
// Groth16 fixture — from groth16-solana v0.2.0 built-in demo circuit
// ---------------------------------------------------------------------------

const DEMO_COMPUTATION_ID = Buffer.from([
  23, 199, 119, 83, 7, 207, 206, 48, 5, 163, 228, 138, 241, 216, 145, 91,
  193, 28, 25, 123, 203, 251, 9, 53, 2, 35, 72, 231, 68, 94, 197, 56,
]);

// Valid Groth16 proof: [neg(proof_a) | proof_b | proof_c] (256 bytes).
// neg(proof_a).x = PROOF[0..32] (unchanged)
// neg(proof_a).y = BN254_PRIME - PROOF[32..64]
// BN254_PRIME = 21888242871839275222246405745257275088696311157297823662689037894645226208583
// y_raw = [20,24,216,15,209,175,106,75,147,236,90,101,123,219,245,151,
//          209,202,218,104,148,8,32,254,243,191,218,122,42,81,193,84]
// neg_y  = [28,75,118,99,15,130,53,222,36,99,235,81,5,165,98,197,
//           197,182,144,40,212,105,169,142,72,96,177,156,174,43,59,243]
const VALID_PROOF = Buffer.from([
  // proof_a negated
  45, 206, 255, 166, 152, 55, 128, 138, 79, 217, 145, 164, 25, 74, 120, 234,
  234, 217, 68, 149, 162, 44, 133, 120, 184, 205, 12, 44, 175, 98, 168, 172,
  28, 75, 118, 99, 15, 130, 53, 222, 36, 99, 235, 81, 5, 165, 98, 197,
  197, 182, 144, 40, 212, 105, 169, 142, 72, 96, 177, 156, 174, 43, 59, 243,
  // proof_b
  40, 57, 233, 205, 180, 46, 35, 111, 215, 5, 23, 93, 12, 71, 118, 225,
  7, 46, 247, 147, 47, 130, 106, 189, 184, 80, 146, 103, 141, 52, 242, 25,
  0, 203, 124, 176, 110, 34, 151, 212, 66, 180, 238, 151, 236, 189, 133, 209,
  17, 137, 205, 183, 168, 196, 92, 159, 75, 174, 81, 168, 18, 86, 176, 56,
  16, 26, 210, 20, 18, 81, 122, 142, 104, 62, 251, 169, 98, 141, 21, 253,
  50, 130, 182, 15, 33, 109, 228, 31, 79, 183, 88, 147, 174, 108, 4, 22,
  14, 129, 168, 6, 80, 246, 254, 100, 218, 131, 94, 49, 247, 211, 3, 245,
  22, 200, 177, 91, 60, 144, 147, 174, 90, 17, 19, 189, 62, 147, 152, 18,
  // proof_c
  41, 139, 183, 208, 246, 198, 118, 127, 89, 160, 9, 27, 61, 26, 123, 180,
  221, 108, 17, 166, 47, 115, 82, 48, 132, 139, 253, 65, 152, 92, 209, 53,
  37, 25, 83, 61, 252, 42, 181, 243, 16, 21, 2, 199, 123, 96, 218, 151,
  253, 86, 69, 181, 202, 109, 64, 129, 124, 254, 192, 25, 177, 199, 26, 50,
]);

// 9 public inputs (32 bytes each) from groth16-solana PUBLIC_INPUTS fixture.
const PUBLIC_INPUTS: Buffer[] = [
  Buffer.from([34,238,251,182,234,248,214,189,46,67,42,25,71,58,145,58,61,28,116,110,60,17,82,149,178,187,160,211,37,226,174,231]),
  Buffer.from([0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,51,152,17,147]),
  Buffer.from([4,247,199,87,230,85,103,90,28,183,95,100,200,46,3,158,247,196,173,146,207,167,108,33,199,18,13,204,198,101,223,186]),
  Buffer.from([0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,7,49,65,41]),
  Buffer.from([7,130,55,65,197,232,175,217,44,151,149,225,75,86,158,105,43,229,65,87,51,150,168,243,176,175,11,203,180,149,72,103]),
  Buffer.from([46,93,177,62,42,66,223,153,51,193,146,49,154,41,69,198,224,13,87,80,222,171,37,141,0,1,50,172,18,28,213,213]),
  Buffer.from([40,141,45,3,180,200,250,112,108,94,35,143,82,63,125,9,147,37,191,75,62,221,138,20,166,151,219,237,254,58,230,189]),
  Buffer.from([33,100,143,241,11,251,73,141,229,57,129,168,83,23,235,147,138,225,177,250,13,97,226,162,6,232,52,95,128,84,90,202]),
  Buffer.from([25,178,1,208,219,169,222,123,113,202,165,77,183,98,103,237,187,93,178,95,169,156,38,100,125,218,104,94,104,119,13,21]),
];

const SONAR_PROGRAM_ID = new PublicKey("EE2sQ2VRa1hY3qjPQ1PEwuPZX6dGwTZwHMCumWrGn3sV");
const ECHO_CALLBACK_ID = new PublicKey("3RBU9G6Mws9nS8bQPg2cVRbS2v7CgsjAvv2MwmTcmbyA");

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function requestPDA(programId: PublicKey, requestId: Buffer): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([Buffer.from("request"), requestId], programId);
}

function resultPDA(programId: PublicKey, requestId: Buffer): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([Buffer.from("result"), requestId], programId);
}

function verifierPDA(programId: PublicKey, computationId: Buffer): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([Buffer.from("verifier"), computationId], programId);
}

const VERIFIER_REGISTRY_ACCOUNT_SIZE = 8 + 32 + 32 + 128 + 1;

function randomId(): Buffer {
  return Buffer.from(Keypair.generate().publicKey.toBytes());
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForSlot(provider: anchor.AnchorProvider, targetSlot: number): Promise<void> {
  for (let attempt = 0; attempt < 30; attempt += 1) {
    const currentSlot = await provider.connection.getSlot();
    if (currentSlot > targetSlot) {
      return;
    }
    await sleep(400);
  }
  assert.fail(`Timed out waiting for slot to advance past ${targetSlot}`);
}

async function expectError(fn: () => Promise<unknown>, code: string): Promise<void> {
  try {
    await fn();
    assert.fail(`Expected "${code}" but transaction succeeded`);
  } catch (err: unknown) {
    if (err instanceof AnchorError) {
      assert.strictEqual(
        err.error.errorCode.code,
        code,
        `Expected "${code}" but got "${err.error.errorCode.code}": ${err.message}`
      );
    } else {
      assert.include(
        String(err),
        code,
        `Expected error to contain "${code}", got: ${String(err)}`
      );
    }
  }
}

// ---------------------------------------------------------------------------
// Test suite
// ---------------------------------------------------------------------------

describe("Sonar ZK Coprocessor — Phase 2.3 Integration Tests", () => {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  let program: anchor.Program<any>;
  let sonarProgramId: PublicKey;
  let echoCallbackId: PublicKey;
  let provider: anchor.AnchorProvider;

  before(async () => {
    provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);

    sonarProgramId = SONAR_PROGRAM_ID;
    echoCallbackId = ECHO_CALLBACK_ID;

    const idl = JSON.parse(
      readFileSync(join(process.cwd(), "target", "idl", "sonar.json"), "utf8")
    );
    // Anchor v0.32: constructor is (idl, provider?, coder?) — program ID comes
    // from idl.address. Passing a PublicKey as the second arg was wrong.
    program = new anchor.Program(idl, provider);

    const sig = await provider.connection.requestAirdrop(
      provider.wallet.publicKey,
      20 * LAMPORTS_PER_SOL
    );
    await provider.connection.confirmTransaction(sig, "confirmed");
  });

  // ==========================================================================
  // ACCESS CONTROL (3 tests)
  // ==========================================================================

  describe("Access Control", () => {
    it("registerVerifier creates and populates verifier registry PDA", async () => {
      const computationId = Buffer.from(Keypair.generate().publicKey.toBytes());
      const vkey = Buffer.from(Array.from({ length: 128 }, (_, index) => (index * 7) % 256));
      const [verifierRegistry] = verifierPDA(sonarProgramId, computationId);

      await program.methods
        .registerVerifier({
          computationId: Array.from(computationId),
          vkey: Array.from(vkey),
        })
        .accounts({
          authority: provider.wallet.publicKey,
          verifierRegistry,
          systemProgram: SystemProgram.programId,
        })
        .rpc();

      const registry = await program.account.verifierRegistry.fetch(verifierRegistry);
      assert.deepEqual(
        Array.from(registry.computationId as number[]),
        Array.from(computationId),
        "computation_id"
      );
      assert.ok(
        (registry.authority as PublicKey).equals(provider.wallet.publicKey),
        "authority"
      );
      assert.deepEqual(Array.from(registry.vkey as number[]), Array.from(vkey), "vkey");

      const accountInfo = await provider.connection.getAccountInfo(verifierRegistry, "confirmed");
      assert.isNotNull(accountInfo, "verifier registry account must exist");
      assert.strictEqual(accountInfo!.data.length, VERIFIER_REGISTRY_ACCOUNT_SIZE, "account size");
    });

    it("rejects refund from a different signer (RefundPayerMismatch)", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();

      await program.methods
        .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(slot + 5000), fee: new anchor.BN(100_000) })
        .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
        .rpc();

      const thief = Keypair.generate();
      const airdrop = await provider.connection.requestAirdrop(thief.publicKey, LAMPORTS_PER_SOL);
      await provider.connection.confirmTransaction(airdrop, "confirmed");

      await expectError(async () => {
        await program.methods
          .refund()
          .accounts({ requestMetadata: reqPda, payer: thief.publicKey })
          .signers([thief])
          .rpc();
      }, "RefundPayerMismatch");
    });

    it("accepts a request whose callback program is executable", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();

      await program.methods
        .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(slot + 5000), fee: new anchor.BN(100_000) })
        .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
        .rpc();

      const meta = await program.account.requestMetadata.fetch(reqPda);
      assert.ok(meta !== null);
    });

    it("rejects a request whose callback program is not executable (CallbackProgramNotExecutable)", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();
      const nonExec = Keypair.generate().publicKey;

      await expectError(async () => {
        await program.methods
          .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(slot + 5000), fee: new anchor.BN(100_000) })
          .accounts({ payer: provider.wallet.publicKey, callbackProgram: nonExec, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
          .rpc();
      }, "CallbackProgramNotExecutable");
    });
  });

  // ==========================================================================
  // REQUEST FLOW (3 tests)
  // ==========================================================================

  describe("Request Flow", () => {
    it("request creates and populates request_metadata PDA", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();
      const deadline = slot + 5000;
      const fee = 200_000;

      await program.methods
        .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.from([1, 2, 3]), deadline: new anchor.BN(deadline), fee: new anchor.BN(fee) })
        .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
        .rpc();

      const meta = await program.account.requestMetadata.fetch(reqPda);
      assert.deepEqual(Array.from(meta.requestId as number[]), Array.from(rid), "request_id");
      assert.ok((meta.payer as PublicKey).equals(provider.wallet.publicKey), "payer");
      assert.ok((meta.callbackProgram as PublicKey).equals(echoCallbackId), "callback_program");
      assert.ok((meta.resultAccount as PublicKey).equals(resPda), "result_account");
      assert.deepEqual(Array.from(meta.computationId as number[]), Array.from(DEMO_COMPUTATION_ID), "computation_id");
      assert.ok((meta.fee as anchor.BN).eq(new anchor.BN(fee)), "fee");
      assert.ok("pending" in (meta.status as object), "status=Pending");
    });

    it("request transfers fee lamports from payer to PDA", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();
      const fee = 500_000;

      const before = await provider.connection.getBalance(provider.wallet.publicKey);

      await program.methods
        .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(slot + 5000), fee: new anchor.BN(fee) })
        .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
        .rpc();

      const after = await provider.connection.getBalance(provider.wallet.publicKey);
      const pdaBal = await provider.connection.getBalance(reqPda);

      assert.isAbove(before - after, fee - 20_000, "payer lost at least fee");
      assert.isAbove(pdaBal, fee - 1, "PDA holds at least fee lamports");
    });

    it("rejects request with deadline in the past (DeadlinePassed)", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();

      await expectError(async () => {
        await program.methods
          .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(slot), fee: new anchor.BN(100_000) })
          .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
          .rpc();
      }, "DeadlinePassed");
    });
  });

  // ==========================================================================
  // CALLBACK FLOW (5 tests)
  // ==========================================================================

  describe("Callback Flow", () => {
    it("callback with valid proof writes result and marks request Completed", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();
      const resultPayload = Buffer.from([0xde, 0xad, 0xbe, 0xef]);

      await program.methods
        .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(slot + 5000), fee: new anchor.BN(1_000_000) })
        .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
        .rpc();

      await program.methods
        .callback({ proof: VALID_PROOF, publicInputs: PUBLIC_INPUTS, result: resultPayload })
        .accounts({ requestMetadata: reqPda, resultAccount: resPda, prover: provider.wallet.publicKey, callbackProgram: echoCallbackId })
        .remainingAccounts([])
        .rpc();

      const meta = await program.account.requestMetadata.fetch(reqPda);
      assert.ok("completed" in (meta.status as object), "status=Completed");

      const res = await program.account.resultAccount.fetch(resPda);
      assert.ok(res.isSet as boolean, "result_account.is_set");
      assert.deepEqual(Array.from(res.result as number[]), Array.from(resultPayload), "result payload");
    });

    it("callback with invalid proof fails with ProofVerificationFailed", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();

      await program.methods
        .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(slot + 5000), fee: new anchor.BN(100_000) })
        .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
        .rpc();

      const badProof = Buffer.from(VALID_PROOF);
      badProof[0] ^= 0xff; // corrupt first byte of proof_a

      await expectError(async () => {
        await program.methods
          .callback({ proof: badProof, publicInputs: PUBLIC_INPUTS, result: Buffer.alloc(0) })
          .accounts({ requestMetadata: reqPda, resultAccount: resPda, prover: provider.wallet.publicKey, callbackProgram: echoCallbackId })
          .remainingAccounts([])
          .rpc();
      }, "ProofVerificationFailed");
    });

    it("callback with mismatched result_account fails with InvalidRequestId", async () => {
      const idA = randomId();
      const idB = randomId();
      const [reqA] = requestPDA(sonarProgramId, idA);
      const [resA] = resultPDA(sonarProgramId, idA);
      const [_reqB, ] = requestPDA(sonarProgramId, idB);
      const [resB] = resultPDA(sonarProgramId, idB);
      const slot = await provider.connection.getSlot();

      for (const [id, req, res] of [[idA, reqA, resA], [idB, _reqB, resB]] as [Buffer, PublicKey, PublicKey][]) {
        await program.methods
          .request({ requestId: Array.from(id), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(slot + 5000), fee: new anchor.BN(100_000) })
          .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: req, resultAccount: res, systemProgram: SystemProgram.programId })
          .rpc();
      }

      await expectError(async () => {
        await program.methods
          .callback({ proof: VALID_PROOF, publicInputs: PUBLIC_INPUTS, result: Buffer.alloc(0) })
          .accounts({ requestMetadata: reqA, resultAccount: resB, prover: provider.wallet.publicKey, callbackProgram: echoCallbackId })
          .remainingAccounts([])
          .rpc();
      }, "InvalidRequestId");
    });

    it("callback after deadline fails with DeadlinePassed", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();
      const deadline = slot + 3;

      await program.methods
        .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(deadline), fee: new anchor.BN(100_000) })
        .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
        .rpc();

      await waitForSlot(provider, deadline);

      await expectError(async () => {
        await program.methods
          .callback({ proof: VALID_PROOF, publicInputs: PUBLIC_INPUTS, result: Buffer.alloc(0) })
          .accounts({ requestMetadata: reqPda, resultAccount: resPda, prover: provider.wallet.publicKey, callbackProgram: echoCallbackId })
          .remainingAccounts([])
          .rpc();
      }, "DeadlinePassed");
    });

    it("successful callback transfers fee lamports to prover", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();
      const fee = 2_000_000; // >> typical tx cost (~5 000 lamports)

      await program.methods
        .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(slot + 5000), fee: new anchor.BN(fee) })
        .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
        .rpc();

      const before = await provider.connection.getBalance(provider.wallet.publicKey);

      await program.methods
        .callback({ proof: VALID_PROOF, publicInputs: PUBLIC_INPUTS, result: Buffer.alloc(0) })
        .accounts({ requestMetadata: reqPda, resultAccount: resPda, prover: provider.wallet.publicKey, callbackProgram: echoCallbackId })
        .remainingAccounts([])
        .rpc();

      const after = await provider.connection.getBalance(provider.wallet.publicKey);
      assert.isAbove(after, before, "prover balance should increase after receiving fee");
    });
  });

  // ==========================================================================
  // REFUND FLOW (2 tests)
  // ==========================================================================

  describe("Refund Flow", () => {
    it("refund before deadline fails with DeadlineNotReached", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();

      await program.methods
        .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(slot + 5000), fee: new anchor.BN(100_000) })
        .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
        .rpc();

      await expectError(async () => {
        await program.methods
          .refund()
          .accounts({ requestMetadata: reqPda, payer: provider.wallet.publicKey })
          .rpc();
      }, "DeadlineNotReached");
    });

    it("refund after deadline returns fee to payer and marks Refunded", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();
      const fee = 1_000_000;
      const deadline = slot + 3;

      await program.methods
        .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(deadline), fee: new anchor.BN(fee) })
        .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
        .rpc();

      await waitForSlot(provider, deadline);

      const before = await provider.connection.getBalance(provider.wallet.publicKey);

      await program.methods
        .refund()
        .accounts({ requestMetadata: reqPda, payer: provider.wallet.publicKey })
        .rpc();

      const after = await provider.connection.getBalance(provider.wallet.publicKey);
      assert.isAbove(after - before, fee - 50_000, "payer should receive back at least fee - tx_cost");

      const meta = await program.account.requestMetadata.fetch(reqPda);
      assert.ok("refunded" in (meta.status as object), "status=Refunded");
    });
  });

  // ==========================================================================
  // EDGE CASES (4 tests)
  // ==========================================================================

  describe("Edge Cases", () => {
    it("second callback on completed request fails with RequestNotPending", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();

      await program.methods
        .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(slot + 5000), fee: new anchor.BN(1_000_000) })
        .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
        .rpc();

      await program.methods
        .callback({ proof: VALID_PROOF, publicInputs: PUBLIC_INPUTS, result: Buffer.from([0x01]) })
        .accounts({ requestMetadata: reqPda, resultAccount: resPda, prover: provider.wallet.publicKey, callbackProgram: echoCallbackId })
        .remainingAccounts([])
        .rpc();

      await expectError(async () => {
        await program.methods
          .callback({ proof: VALID_PROOF, publicInputs: PUBLIC_INPUTS, result: Buffer.from([0x02]) })
          .accounts({ requestMetadata: reqPda, resultAccount: resPda, prover: provider.wallet.publicKey, callbackProgram: echoCallbackId })
          .remainingAccounts([])
          .rpc();
      }, "RequestNotPending");
    });

    it("callback with wrong result PDA fails with InvalidRequestId", async () => {
      const idX = randomId();
      const idY = randomId();
      const [reqX] = requestPDA(sonarProgramId, idX);
      const [resX] = resultPDA(sonarProgramId, idX);
      const [reqY] = requestPDA(sonarProgramId, idY);
      const [resY] = resultPDA(sonarProgramId, idY);
      const slot = await provider.connection.getSlot();

      for (const [id, req, res] of [[idX, reqX, resX], [idY, reqY, resY]] as [Buffer, PublicKey, PublicKey][]) {
        await program.methods
          .request({ requestId: Array.from(id), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(slot + 5000), fee: new anchor.BN(100_000) })
          .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: req, resultAccount: res, systemProgram: SystemProgram.programId })
          .rpc();
      }

      await expectError(async () => {
        await program.methods
          .callback({ proof: VALID_PROOF, publicInputs: PUBLIC_INPUTS, result: Buffer.alloc(0) })
          .accounts({ requestMetadata: reqX, resultAccount: resY, prover: provider.wallet.publicKey, callbackProgram: echoCallbackId })
          .remainingAccounts([])
          .rpc();
      }, "InvalidRequestId");
    });

    it("request handles non-trivial input size (700 bytes)", async () => {
      const rid = randomId();
      const [reqPda] = requestPDA(sonarProgramId, rid);
      const [resPda] = resultPDA(sonarProgramId, rid);
      const slot = await provider.connection.getSlot();
      const bigInputs = Buffer.from(Array.from({ length: 700 }, (_, i) => i % 256));

      await program.methods
        .request({ requestId: Array.from(rid), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: bigInputs, deadline: new anchor.BN(slot + 5000), fee: new anchor.BN(100_000) })
        .preInstructions([
          ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
        ])
        .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
        .rpc();

      const meta = await program.account.requestMetadata.fetch(reqPda);
      assert.ok("pending" in (meta.status as object), "status=Pending");
    });

    it("multiple concurrent requests all succeed with correct metadata", async () => {
      const slot = await provider.connection.getSlot();
      const ids = [randomId(), randomId(), randomId(), randomId()];

      await Promise.all(
        ids.map((id) => {
          const [reqPda] = requestPDA(sonarProgramId, id);
          const [resPda] = resultPDA(sonarProgramId, id);
          return program.methods
            .request({ requestId: Array.from(id), computationId: Array.from(DEMO_COMPUTATION_ID), inputs: Buffer.alloc(0), deadline: new anchor.BN(slot + 5000), fee: new anchor.BN(100_000) })
            .accounts({ payer: provider.wallet.publicKey, callbackProgram: echoCallbackId, requestMetadata: reqPda, resultAccount: resPda, systemProgram: SystemProgram.programId })
            .rpc();
        })
      );

      for (const id of ids) {
        const [reqPda] = requestPDA(sonarProgramId, id);
        const meta = await program.account.requestMetadata.fetch(reqPda);
        assert.deepEqual(Array.from(meta.requestId as number[]), Array.from(id), "concurrent request_id mismatch");
      }
    });
  });

  after(() => {
    console.log("\nsonar integration checks passed");
  });
});
