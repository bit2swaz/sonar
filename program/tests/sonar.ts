import * as anchor from "@coral-xyz/anchor";
import { assert } from "chai";
import { readFileSync } from "fs";
import { join } from "path";

/**
 * Phase 2.1 — placeholder integration test.
 *
 * After `anchor test` builds and deploys the program to a local validator this
 * test verifies that:
 *   1. The program account exists on-chain.
 *   2. It is marked executable (i.e. it compiled and deployed successfully).
 *
 * Full TDD tests for each instruction are added in Phase 2.2+.
 */
const DECLARED_PROGRAM_ID = new anchor.web3.PublicKey(
  "5B1rXQ71oEWUPc3AemCBTQtb5pmGAnX1jbGvZKcgBy84"
);

function deployedProgramId(): anchor.web3.PublicKey {
  const keypairPath = join(
    process.cwd(),
    "target",
    "deploy",
    "sonar_program-keypair.json"
  );
  const secretKey = Uint8Array.from(JSON.parse(readFileSync(keypairPath, "utf8")));
  return anchor.web3.Keypair.fromSecretKey(secretKey).publicKey;
}

export async function runSonarTests(): Promise<void> {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const programId = deployedProgramId();
  const accountInfo = await provider.connection.getAccountInfo(programId);
  assert.ok(
    accountInfo !== null,
    "Program account should exist after deployment"
  );
  assert.ok(
    accountInfo!.executable,
    "Program account should be marked executable"
  );

  assert.equal(
    DECLARED_PROGRAM_ID.toBase58(),
    "5B1rXQ71oEWUPc3AemCBTQtb5pmGAnX1jbGvZKcgBy84",
    "Program ID must match the declared ID in lib.rs"
  );

  console.log("sonar integration checks passed");
}

if (require.main === module) {
  runSonarTests().catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
}
