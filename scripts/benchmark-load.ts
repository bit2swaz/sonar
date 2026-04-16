import * as anchor from "@coral-xyz/anchor";
import type { Idl } from "@coral-xyz/anchor";
import {
  clusterApiUrl,
  Connection,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
  type Commitment,
} from "@solana/web3.js";
import { randomBytes } from "crypto";
import { existsSync } from "fs";
import { homedir } from "os";
import { resolve } from "path";
import { performance } from "perf_hooks";

import sonarIdl from "../target/idl/sonar.json";
import type { Sonar } from "../target/types/sonar";

const DEFAULT_RPC_URL = clusterApiUrl("devnet");
const DEFAULT_WALLET_PATH = resolve(homedir(), ".config/solana/id.json");
const DEFAULT_PROGRAM_ID = new PublicKey("Gf7RSZYmfNJ5kv2AJvcv5rjCANP6ePExJR19D91MECLY");
const DEFAULT_CALLBACK_PROGRAM_ID = new PublicKey("J7jsJVQz6xbWFhyxRbzk7nH5ALhStztUNR1nPupnyjxS");
const DEFAULT_REQUEST_FEE_LAMPORTS = 100_000;
const DEFAULT_DEADLINE_SLOT_DELTA = 5_000;
const DEFAULT_COMMITMENT: Commitment = "confirmed";

const DEMO_COMPUTATION_ID = Buffer.from([
  23, 199, 119, 83, 7, 207, 206, 48, 5, 163, 228, 138, 241, 216, 145, 91,
  193, 28, 25, 123, 203, 251, 9, 53, 2, 35, 72, 231, 68, 94, 197, 56,
]);

const HISTORICAL_AVG_COMPUTATION_ID = Buffer.from([
  180, 134, 237, 198, 23, 219, 85, 143, 84, 245, 61, 62, 222, 122, 82, 179,
  3, 201, 204, 111, 144, 62, 32, 159, 91, 227, 160, 78, 252, 195, 98, 100,
]);

type SupportedComputation = "demo" | "historical-avg";

type CliOptions = {
  requests: number;
  rpcUrl: string;
  walletPath: string;
  programId: PublicKey;
  callbackProgramId: PublicKey;
  feeLamports: number;
  deadlineSlotDelta: number;
  computation: SupportedComputation;
  inputsHex?: string;
  observedAccount?: PublicKey;
  fromSlot?: number;
  toSlot?: number;
};

type SubmitResult = {
  index: number;
  signature: string;
  requestId: Buffer;
  requestMetadata: PublicKey;
  resultAccount: PublicKey;
  elapsedMs: number;
};

function usage(): string {
  return [
    "Usage:",
    "  npx ts-node --transpile-only scripts/benchmark-load.ts --requests 50 [options]",
    "",
    "Required:",
    "  --requests <count>                Number of concurrent Sonar requests to fire",
    "",
    "Optional:",
    `  --rpc-url <url>                   Solana RPC URL (default: ${DEFAULT_RPC_URL})`,
    `  --wallet <path>                   Wallet keypair path (default: ${DEFAULT_WALLET_PATH})`,
    `  --program-id <pubkey>             Sonar program ID (default: ${DEFAULT_PROGRAM_ID.toBase58()})`,
    `  --callback-program <pubkey>       Callback program ID (default: ${DEFAULT_CALLBACK_PROGRAM_ID.toBase58()})`,
    `  --fee-lamports <lamports>         Request fee in lamports (default: ${DEFAULT_REQUEST_FEE_LAMPORTS})`,
    `  --deadline-slots <slots>          Slot delta added to the current slot (default: ${DEFAULT_DEADLINE_SLOT_DELTA})`,
    "  --computation <demo|historical-avg>",
    "                                    Computation to request (default: demo)",
    "  --inputs-hex <hex>                Raw request inputs as hex bytes (demo mode override)",
    "  --observed-account <pubkey>       Historical-average input account",
    "  --from-slot <slot>                Historical-average input start slot",
    "  --to-slot <slot>                  Historical-average input end slot",
    "  --help                            Show this message",
  ].join("\n");
}

function fail(message: string): never {
  throw new Error(message);
}

function parsePositiveInteger(value: string, flagName: string): number {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    fail(`${flagName} must be a positive integer, got \"${value}\"`);
  }
  return parsed;
}

function parseNonNegativeInteger(value: string, flagName: string): number {
  const parsed = Number.parseInt(value, 10);
  if (!Number.isFinite(parsed) || parsed < 0) {
    fail(`${flagName} must be a non-negative integer, got \"${value}\"`);
  }
  return parsed;
}

function parsePublicKey(value: string, flagName: string): PublicKey {
  try {
    return new PublicKey(value);
  } catch {
    fail(`${flagName} must be a valid base58 public key, got \"${value}\"`);
  }
}

function normalizePath(value: string): string {
  if (value === "~") {
    return homedir();
  }
  if (value.startsWith("~/")) {
    return resolve(homedir(), value.slice(2));
  }
  return resolve(value);
}

function parseArgs(argv: string[]): CliOptions {
  const options: CliOptions = {
    requests: 0,
    rpcUrl: DEFAULT_RPC_URL,
    walletPath: DEFAULT_WALLET_PATH,
    programId: DEFAULT_PROGRAM_ID,
    callbackProgramId: DEFAULT_CALLBACK_PROGRAM_ID,
    feeLamports: DEFAULT_REQUEST_FEE_LAMPORTS,
    deadlineSlotDelta: DEFAULT_DEADLINE_SLOT_DELTA,
    computation: "demo",
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = argv[index + 1];

    switch (arg) {
      case "--help":
      case "-h":
        console.log(usage());
        process.exit(0);
        break;
      case "--requests":
        if (!next) fail("--requests requires a value");
        options.requests = parsePositiveInteger(next, "--requests");
        index += 1;
        break;
      case "--rpc-url":
        if (!next) fail("--rpc-url requires a value");
        options.rpcUrl = next;
        index += 1;
        break;
      case "--wallet":
        if (!next) fail("--wallet requires a value");
        options.walletPath = normalizePath(next);
        index += 1;
        break;
      case "--program-id":
        if (!next) fail("--program-id requires a value");
        options.programId = parsePublicKey(next, "--program-id");
        index += 1;
        break;
      case "--callback-program":
        if (!next) fail("--callback-program requires a value");
        options.callbackProgramId = parsePublicKey(next, "--callback-program");
        index += 1;
        break;
      case "--fee-lamports":
        if (!next) fail("--fee-lamports requires a value");
        options.feeLamports = parsePositiveInteger(next, "--fee-lamports");
        index += 1;
        break;
      case "--deadline-slots":
        if (!next) fail("--deadline-slots requires a value");
        options.deadlineSlotDelta = parsePositiveInteger(next, "--deadline-slots");
        index += 1;
        break;
      case "--computation":
        if (!next) fail("--computation requires a value");
        if (next !== "demo" && next !== "historical-avg") {
          fail(`--computation must be one of: demo, historical-avg (got \"${next}\")`);
        }
        options.computation = next;
        index += 1;
        break;
      case "--inputs-hex":
        if (!next) fail("--inputs-hex requires a value");
        options.inputsHex = next;
        index += 1;
        break;
      case "--observed-account":
        if (!next) fail("--observed-account requires a value");
        options.observedAccount = parsePublicKey(next, "--observed-account");
        index += 1;
        break;
      case "--from-slot":
        if (!next) fail("--from-slot requires a value");
        options.fromSlot = parseNonNegativeInteger(next, "--from-slot");
        index += 1;
        break;
      case "--to-slot":
        if (!next) fail("--to-slot requires a value");
        options.toSlot = parseNonNegativeInteger(next, "--to-slot");
        index += 1;
        break;
      default:
        fail(`Unknown argument: ${arg}\n\n${usage()}`);
    }
  }

  if (options.requests === 0) {
    fail(`--requests is required\n\n${usage()}`);
  }

  if (options.computation === "historical-avg") {
    if (!options.observedAccount || options.fromSlot === undefined || options.toSlot === undefined) {
      fail(
        "historical-avg mode requires --observed-account, --from-slot, and --to-slot"
      );
    }
    if (options.toSlot < options.fromSlot) {
      fail("--to-slot must be greater than or equal to --from-slot");
    }
  }

  if (!existsSync(options.walletPath)) {
    fail(`Wallet file not found at ${options.walletPath}`);
  }

  if (options.inputsHex) {
    const normalized = options.inputsHex.startsWith("0x")
      ? options.inputsHex.slice(2)
      : options.inputsHex;
    if (normalized.length % 2 !== 0 || /[^a-fA-F0-9]/.test(normalized)) {
      fail("--inputs-hex must contain an even number of hexadecimal characters");
    }
    options.inputsHex = normalized.toLowerCase();
  }

  return options;
}

function requestPda(programId: PublicKey, requestId: Buffer): PublicKey {
  return PublicKey.findProgramAddressSync([Buffer.from("request"), requestId], programId)[0];
}

function resultPda(programId: PublicKey, requestId: Buffer): PublicKey {
  return PublicKey.findProgramAddressSync([Buffer.from("result"), requestId], programId)[0];
}

function encodeHistoricalAvgInputs(
  observedAccount: PublicKey,
  fromSlot: number,
  toSlot: number
): Buffer {
  const buffer = Buffer.alloc(48);
  observedAccount.toBuffer().copy(buffer, 0);
  buffer.writeBigUInt64LE(BigInt(fromSlot), 32);
  buffer.writeBigUInt64LE(BigInt(toSlot), 40);
  return buffer;
}

function resolveComputationInputs(options: CliOptions): { computationId: Buffer; inputs: Buffer } {
  if (options.computation === "historical-avg") {
    return {
      computationId: Buffer.from(HISTORICAL_AVG_COMPUTATION_ID),
      inputs: encodeHistoricalAvgInputs(
        options.observedAccount!,
        options.fromSlot!,
        options.toSlot!
      ),
    };
  }

  return {
    computationId: Buffer.from(DEMO_COMPUTATION_ID),
    inputs: options.inputsHex ? Buffer.from(options.inputsHex, "hex") : Buffer.alloc(0),
  };
}

async function ensureExecutableAccount(
  connection: Connection,
  publicKey: PublicKey,
  label: string
): Promise<void> {
  const accountInfo = await connection.getAccountInfo(publicKey, DEFAULT_COMMITMENT);
  if (accountInfo === null) {
    fail(`${label} ${publicKey.toBase58()} does not exist on the target cluster`);
  }
  if (!accountInfo.executable) {
    fail(`${label} ${publicKey.toBase58()} is not marked executable on the target cluster`);
  }
}

async function submitSingleRequest(
  index: number,
  total: number,
  program: anchor.Program<Sonar>,
  provider: anchor.AnchorProvider,
  options: CliOptions,
  blockhash: Awaited<ReturnType<Connection["getLatestBlockhash"]>>,
  currentSlot: number,
  computationId: Buffer,
  inputs: Buffer
): Promise<SubmitResult> {
  const requestId = randomBytes(32);
  const requestMetadata = requestPda(options.programId, requestId);
  const resultAccount = resultPda(options.programId, requestId);
  const startedAt = performance.now();

  const transaction = await program.methods
    .request({
      requestId: Array.from(requestId),
      computationId: Array.from(computationId),
      inputs,
      deadline: new anchor.BN(currentSlot + options.deadlineSlotDelta),
      fee: new anchor.BN(options.feeLamports),
    })
    .accountsPartial({
      payer: provider.wallet.publicKey,
      callbackProgram: options.callbackProgramId,
    })
    .transaction();

  transaction.feePayer = provider.wallet.publicKey;
  transaction.recentBlockhash = blockhash.blockhash;

  const signedTransaction = await provider.wallet.signTransaction(transaction);
  const signature = await provider.connection.sendRawTransaction(signedTransaction.serialize(), {
    skipPreflight: true,
    maxRetries: 0,
    preflightCommitment: DEFAULT_COMMITMENT,
  });

  console.log(`[${index}/${total}] submitted ${signature}`);

  const confirmation = await provider.connection.confirmTransaction(
    {
      signature,
      blockhash: blockhash.blockhash,
      lastValidBlockHeight: blockhash.lastValidBlockHeight,
    },
    DEFAULT_COMMITMENT
  );

  if (confirmation.value.err) {
    throw new Error(
      `[${index}/${total}] confirmation failed for ${signature}: ${JSON.stringify(
        confirmation.value.err
      )}`
    );
  }

  const elapsedMs = Math.round(performance.now() - startedAt);
  console.log(`[${index}/${total}] confirmed ${signature} in ${elapsedMs}ms`);

  return {
    index,
    signature,
    requestId,
    requestMetadata,
    resultAccount,
    elapsedMs,
  };
}

async function main(): Promise<void> {
  const options = parseArgs(process.argv.slice(2));

  process.env.ANCHOR_PROVIDER_URL = options.rpcUrl;
  process.env.ANCHOR_WALLET = options.walletPath;

  const envProvider = anchor.AnchorProvider.env();
  const connection = new Connection(options.rpcUrl, {
    commitment: DEFAULT_COMMITMENT,
    confirmTransactionInitialTimeout: 120_000,
  });
  const provider = new anchor.AnchorProvider(connection, envProvider.wallet, {
    commitment: DEFAULT_COMMITMENT,
    preflightCommitment: DEFAULT_COMMITMENT,
  });
  anchor.setProvider(provider);

  const idlWithProgramId = {
    ...(sonarIdl as Idl & { address?: string }),
    address: options.programId.toBase58(),
  };
  const program = new anchor.Program<Sonar>(idlWithProgramId as Sonar, provider);

  await ensureExecutableAccount(provider.connection, options.programId, "Sonar program");
  await ensureExecutableAccount(
    provider.connection,
    options.callbackProgramId,
    "Callback program"
  );

  const { computationId, inputs } = resolveComputationInputs(options);
  const payerBalanceLamports = await provider.connection.getBalance(
    provider.wallet.publicKey,
    DEFAULT_COMMITMENT
  );
  const currentSlot = await provider.connection.getSlot(DEFAULT_COMMITMENT);
  const latestBlockhash = await provider.connection.getLatestBlockhash(DEFAULT_COMMITMENT);

  console.log(`RPC URL: ${options.rpcUrl}`);
  console.log(`Program ID: ${options.programId.toBase58()}`);
  console.log(`Payer: ${provider.wallet.publicKey.toBase58()}`);
  console.log(
    `Payer balance: ${(payerBalanceLamports / LAMPORTS_PER_SOL).toFixed(4)} SOL (${payerBalanceLamports} lamports)`
  );
  console.log(`Callback program: ${options.callbackProgramId.toBase58()}`);
  console.log(`Computation: ${options.computation}`);
  console.log(`Concurrent requests: ${options.requests}`);
  console.log(`Current slot: ${currentSlot}`);
  console.log(`Deadline slot delta: ${options.deadlineSlotDelta}`);
  console.log(`Request fee: ${options.feeLamports} lamports`);
  console.log(`Input bytes: ${inputs.length}`);
  console.log("Starting concurrent request flood...");

  const startedAt = performance.now();
  const tasks = Array.from({ length: options.requests }, (_, offset) =>
    submitSingleRequest(
      offset + 1,
      options.requests,
      program,
      provider,
      options,
      latestBlockhash,
      currentSlot,
      computationId,
      inputs
    )
  );

  const settled = await Promise.allSettled(tasks);
  const successes = settled.filter(
    (result): result is PromiseFulfilledResult<SubmitResult> => result.status === "fulfilled"
  );
  const failures = settled.filter(
    (result): result is PromiseRejectedResult => result.status === "rejected"
  );
  const totalElapsedMs = Math.round(performance.now() - startedAt);

  console.log(
    `Finished load submission in ${totalElapsedMs}ms with ${successes.length}/${options.requests} confirmed requests.`
  );

  if (successes.length > 0) {
    const averageLatencyMs = Math.round(
      successes.reduce((sum, result) => sum + result.value.elapsedMs, 0) / successes.length
    );
    console.log(`Average confirmation latency: ${averageLatencyMs}ms`);
  }

  if (failures.length > 0) {
    console.error(`Encountered ${failures.length} failed request(s):`);
    failures.forEach((failure, failureIndex) => {
      console.error(`  ${failureIndex + 1}. ${String(failure.reason)}`);
    });
  }

  console.log(
    'open Grafana to observe the p50 and p99 metrics for request-to-queue and round-trip latencies.'
  );

  if (failures.length > 0) {
    process.exitCode = 1;
  }
}

main().catch((error) => {
  console.error(`benchmark-load failed: ${error instanceof Error ? error.message : String(error)}`);
  console.log(
    'open Grafana to observe the p50 and p99 metrics for request-to-queue and round-trip latencies.'
  );
  process.exitCode = 1;
});
