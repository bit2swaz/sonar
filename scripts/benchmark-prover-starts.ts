import * as anchor from "@coral-xyz/anchor";
import type { Idl } from "@coral-xyz/anchor";
import {
  Connection,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
  type Commitment,
} from "@solana/web3.js";
import { createHash, randomBytes } from "crypto";
import { existsSync, readFileSync } from "fs";
import { homedir } from "os";
import { resolve } from "path";
import { performance } from "perf_hooks";

import sonarIdl from "../target/idl/sonar.json";
import type { Sonar } from "../target/types/sonar";

const DEFAULT_RPC_URL =
  process.env.SOLANA_RPC_URL ?? "https://solana-devnet.core.chainstack.com/51a4443c8b33222e5327f331e007ec91";
const DEFAULT_WALLET_PATH = resolve(homedir(), ".config/solana/id.json");
const DEFAULT_PROGRAM_ID = new PublicKey("Gf7RSZYmfNJ5kv2AJvcv5rjCANP6ePExJR19D91MECLY");
const DEFAULT_CALLBACK_PROGRAM_ID = new PublicKey("J7jsJVQz6xbWFhyxRbzk7nH5ALhStztUNR1nPupnyjxS");
const DEFAULT_REQUEST_FEE_LAMPORTS = 100_000;
const DEFAULT_DEADLINE_SLOT_DELTA = 5_000;
const DEFAULT_TIMEOUT_SECONDS = 900;
const DEFAULT_POLL_INTERVAL_MS = 2_000;
const DEFAULT_SLOT_WINDOW = 128;
const DEFAULT_COMMITMENT: Commitment = "confirmed";
const FIBONACCI_ELF_PATH = resolve(
  process.cwd(),
  "programs/fibonacci/elf/fibonacci-program"
);
const HISTORICAL_AVG_COMPUTATION_ID = Buffer.from([
  180, 134, 237, 198, 23, 219, 85, 143, 84, 245, 61, 62, 222, 122, 82, 179,
  3, 201, 204, 111, 144, 62, 32, 159, 91, 227, 160, 78, 252, 195, 98, 100,
]);

type SupportedComputation = "historical-avg" | "fibonacci";

type CliOptions = {
  rpcUrl: string;
  walletPath: string;
  programId: PublicKey;
  callbackProgramId: PublicKey;
  feeLamports: number;
  deadlineSlotDelta: number;
  timeoutSeconds: number;
  pollIntervalMs: number;
  computation: SupportedComputation;
  observedAccount?: PublicKey;
  fromSlot?: number;
  toSlot?: number;
  slotWindow: number;
  fibonacciN: number;
};

type ResolvedInputs = {
  computationLabel: string;
  computationId: Buffer;
  inputs: Buffer;
};

type RequestRunResult = {
  signature: string;
  resultAccount: PublicKey;
  wallClockMs: number;
};

type ResultAccountState = {
  isSet: boolean;
  writtenAt?: anchor.BN | null;
  written_at?: anchor.BN | null;
};

function usage(): string {
  return [
    "Usage:",
    "  npx ts-node --transpile-only scripts/benchmark-prover-starts.ts [options]",
    "",
    "Optional:",
    `  --rpc-url <url>                   Solana RPC URL (default: ${DEFAULT_RPC_URL})`,
    `  --wallet <path>                   Wallet keypair path (default: ${DEFAULT_WALLET_PATH})`,
    `  --program-id <pubkey>             Sonar program ID (default: ${DEFAULT_PROGRAM_ID.toBase58()})`,
    `  --callback-program <pubkey>       Callback program ID (default: ${DEFAULT_CALLBACK_PROGRAM_ID.toBase58()})`,
    `  --fee-lamports <lamports>         Request fee in lamports (default: ${DEFAULT_REQUEST_FEE_LAMPORTS})`,
    `  --deadline-slots <slots>          Slot delta added to the current slot (default: ${DEFAULT_DEADLINE_SLOT_DELTA})`,
    `  --timeout-seconds <seconds>       Max callback wait per request (default: ${DEFAULT_TIMEOUT_SECONDS})`,
    `  --poll-interval-ms <ms>           Result-account poll interval (default: ${DEFAULT_POLL_INTERVAL_MS})`,
    "  --computation <historical-avg|fibonacci>",
    "                                    Benchmark computation to request (default: historical-avg)",
    "  --observed-account <pubkey>       Historical-average input account (defaults to payer)",
    "  --from-slot <slot>                Historical-average input start slot",
    "  --to-slot <slot>                  Historical-average input end slot",
    `  --slot-window <slots>             Default historical-average slot window when from/to are omitted (default: ${DEFAULT_SLOT_WINDOW})`,
    "  --fib-n <n>                       Fibonacci input for the prover guest (default: 30)",
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
    rpcUrl: DEFAULT_RPC_URL,
    walletPath: DEFAULT_WALLET_PATH,
    programId: DEFAULT_PROGRAM_ID,
    callbackProgramId: DEFAULT_CALLBACK_PROGRAM_ID,
    feeLamports: DEFAULT_REQUEST_FEE_LAMPORTS,
    deadlineSlotDelta: DEFAULT_DEADLINE_SLOT_DELTA,
    timeoutSeconds: DEFAULT_TIMEOUT_SECONDS,
    pollIntervalMs: DEFAULT_POLL_INTERVAL_MS,
    computation: "historical-avg",
    slotWindow: DEFAULT_SLOT_WINDOW,
    fibonacciN: 30,
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
      case "--timeout-seconds":
        if (!next) fail("--timeout-seconds requires a value");
        options.timeoutSeconds = parsePositiveInteger(next, "--timeout-seconds");
        index += 1;
        break;
      case "--poll-interval-ms":
        if (!next) fail("--poll-interval-ms requires a value");
        options.pollIntervalMs = parsePositiveInteger(next, "--poll-interval-ms");
        index += 1;
        break;
      case "--computation":
        if (!next) fail("--computation requires a value");
        if (next !== "historical-avg" && next !== "fibonacci") {
          fail(`--computation must be one of: historical-avg, fibonacci (got \"${next}\")`);
        }
        options.computation = next;
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
      case "--slot-window":
        if (!next) fail("--slot-window requires a value");
        options.slotWindow = parsePositiveInteger(next, "--slot-window");
        index += 1;
        break;
      case "--fib-n":
        if (!next) fail("--fib-n requires a value");
        options.fibonacciN = parsePositiveInteger(next, "--fib-n");
        index += 1;
        break;
      default:
        fail(`Unknown argument: ${arg}\n\n${usage()}`);
    }
  }

  if (!existsSync(options.walletPath)) {
    fail(`Wallet file not found at ${options.walletPath}`);
  }

  if (options.computation === "historical-avg") {
    const fromConfigured = options.fromSlot !== undefined;
    const toConfigured = options.toSlot !== undefined;
    if (fromConfigured !== toConfigured) {
      fail("--from-slot and --to-slot must be provided together");
    }
    if (
      options.fromSlot !== undefined &&
      options.toSlot !== undefined &&
      options.toSlot < options.fromSlot
    ) {
      fail("--to-slot must be greater than or equal to --from-slot");
    }
  }

  if (options.computation === "fibonacci" && !existsSync(FIBONACCI_ELF_PATH)) {
    fail(`Fibonacci ELF not found at ${FIBONACCI_ELF_PATH}`);
  }

  return options;
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
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

function encodeFibonacciInputs(n: number): Buffer {
  const buffer = Buffer.alloc(4);
  buffer.writeUInt32LE(n, 0);
  return buffer;
}

function resolveFibonacciComputationId(): Buffer {
  const elf = readFileSync(FIBONACCI_ELF_PATH);
  return createHash("sha256").update(elf).digest();
}

async function resolveInputs(
  options: CliOptions,
  payerPublicKey: PublicKey,
  currentSlot: number
): Promise<ResolvedInputs> {
  if (options.computation === "fibonacci") {
    return {
      computationLabel: `fibonacci(n=${options.fibonacciN})`,
      computationId: resolveFibonacciComputationId(),
      inputs: encodeFibonacciInputs(options.fibonacciN),
    };
  }

  const observedAccount = options.observedAccount ?? payerPublicKey;
  const toSlot = options.toSlot ?? currentSlot;
  const fromSlot =
    options.fromSlot ?? Math.max(0, toSlot - options.slotWindow);

  return {
    computationLabel: `historical-avg(account=${observedAccount.toBase58()}, from=${fromSlot}, to=${toSlot})`,
    computationId: Buffer.from(HISTORICAL_AVG_COMPUTATION_ID),
    inputs: encodeHistoricalAvgInputs(observedAccount, fromSlot, toSlot),
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

async function waitForCallbackCompletion(
  program: anchor.Program<Sonar>,
  resultAccount: PublicKey,
  timeoutSeconds: number,
  pollIntervalMs: number,
  label: string
): Promise<void> {
  const deadlineAt = Date.now() + timeoutSeconds * 1000;
  let lastSeenWrittenAt: string | undefined;

  while (Date.now() < deadlineAt) {
    const accountInfo = await program.provider.connection.getAccountInfo(
      resultAccount,
      DEFAULT_COMMITMENT
    );

    if (accountInfo === null) {
      return;
    }

    try {
      const state = (await program.account.resultAccount.fetch(resultAccount)) as ResultAccountState;
      const isSet = Boolean(state.isSet);
      const writtenAt = state.writtenAt ?? state.written_at ?? null;
      if (writtenAt !== null && writtenAt !== undefined) {
        lastSeenWrittenAt = writtenAt.toString();
      }
      if (isSet) {
        return;
      }
    } catch (error: unknown) {
      const message = error instanceof Error ? error.message : String(error);
      if (
        !message.includes("Account does not exist") &&
        !message.includes("Account not found") &&
        !message.includes("Failed to deserialize")
      ) {
        throw error;
      }
    }

    await sleep(pollIntervalMs);
  }

  const suffix = lastSeenWrittenAt ? ` last_written_at=${lastSeenWrittenAt}` : "";
  throw new Error(
    `${label} timed out waiting ${timeoutSeconds}s for callback completion on ${resultAccount.toBase58()}.${suffix}`
  );
}

async function submitAndMeasure(
  label: string,
  program: anchor.Program<Sonar>,
  provider: anchor.AnchorProvider,
  options: CliOptions,
  resolvedInputs: ResolvedInputs
): Promise<RequestRunResult> {
  const requestId = randomBytes(32);
  const resultAccount = resultPda(options.programId, requestId);
  const requestStartedAt = performance.now();
  const startIso = new Date().toISOString();

  const currentSlot = await provider.connection.getSlot(DEFAULT_COMMITMENT);
  const latestBlockhash = await provider.connection.getLatestBlockhash(DEFAULT_COMMITMENT);

  const transaction = await program.methods
    .request({
      requestId: Array.from(requestId),
      computationId: Array.from(resolvedInputs.computationId),
      inputs: resolvedInputs.inputs,
      deadline: new anchor.BN(currentSlot + options.deadlineSlotDelta),
      fee: new anchor.BN(options.feeLamports),
    })
    .accountsPartial({
      payer: provider.wallet.publicKey,
      callbackProgram: options.callbackProgramId,
    })
    .transaction();

  transaction.feePayer = provider.wallet.publicKey;
  transaction.recentBlockhash = latestBlockhash.blockhash;

  const signedTransaction = await provider.wallet.signTransaction(transaction);
  const signature = await provider.connection.sendRawTransaction(signedTransaction.serialize(), {
    skipPreflight: true,
    maxRetries: 0,
    preflightCommitment: DEFAULT_COMMITMENT,
  });

  console.log(`[${label}] submitted ${signature} at ${startIso}`);

  const txConfirmation = await provider.connection.confirmTransaction(
    {
      signature,
      blockhash: latestBlockhash.blockhash,
      lastValidBlockHeight: latestBlockhash.lastValidBlockHeight,
    },
    DEFAULT_COMMITMENT
  );

  if (txConfirmation.value.err) {
    throw new Error(
      `[${label}] request transaction ${signature} failed: ${JSON.stringify(txConfirmation.value.err)}`
    );
  }

  console.log(`[${label}] request transaction confirmed ${signature}; waiting for callback...`);

  await waitForCallbackCompletion(
    program,
    resultAccount,
    options.timeoutSeconds,
    options.pollIntervalMs,
    label
  );

  const completedIso = new Date().toISOString();
  const wallClockMs = Math.round(performance.now() - requestStartedAt);
  console.log(`[${label}] callback completed for ${signature} at ${completedIso} in ${wallClockMs}ms`);

  return {
    signature,
    resultAccount,
    wallClockMs,
  };
}

function average(values: number[]): number {
  if (values.length === 0) {
    return 0;
  }
  return values.reduce((sum, value) => sum + value, 0) / values.length;
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

  const payerBalanceLamports = await provider.connection.getBalance(
    provider.wallet.publicKey,
    DEFAULT_COMMITMENT
  );
  const currentSlot = await provider.connection.getSlot(DEFAULT_COMMITMENT);
  const resolvedInputs = await resolveInputs(options, provider.wallet.publicKey, currentSlot);

  console.log(`RPC URL: ${options.rpcUrl}`);
  console.log(`Program ID: ${options.programId.toBase58()}`);
  console.log(`Payer: ${provider.wallet.publicKey.toBase58()}`);
  console.log(
    `Payer balance: ${(payerBalanceLamports / LAMPORTS_PER_SOL).toFixed(4)} SOL (${payerBalanceLamports} lamports)`
  );
  console.log(`Callback program: ${options.callbackProgramId.toBase58()}`);
  console.log(`Computation: ${resolvedInputs.computationLabel}`);
  console.log(`Timeout per request: ${options.timeoutSeconds}s`);
  console.log(`Poll interval: ${options.pollIntervalMs}ms`);
  console.log("Starting cold request benchmark...");

  const coldResult = await submitAndMeasure(
    "cold-1",
    program,
    provider,
    options,
    resolvedInputs
  );

  console.log("Cold request finished. Starting 10 concurrent warm requests immediately...");

  const warmBatchStartedAt = performance.now();
  const warmResults = await Promise.all(
    Array.from({ length: 10 }, (_, index) =>
      submitAndMeasure(`warm-${index + 1}`, program, provider, options, resolvedInputs)
    )
  );
  const warmBatchElapsedMs = Math.round(performance.now() - warmBatchStartedAt);
  const warmAverageMs = Math.round(
    average(warmResults.map((result) => result.wallClockMs))
  );
  const deltaMs = coldResult.wallClockMs - warmAverageMs;
  const ratio = warmAverageMs === 0 ? 0 : coldResult.wallClockMs / warmAverageMs;

  console.log("");
  console.log("=== Prover cold vs warm start comparison ===");
  console.log(`Cold start wall-clock time: ${coldResult.wallClockMs}ms`);
  console.log(`Warm start average wall-clock time (10 requests): ${warmAverageMs}ms`);
  console.log(`Warm batch total elapsed time: ${warmBatchElapsedMs}ms`);
  console.log(`Cold - warm average delta: ${deltaMs}ms`);
  console.log(`Cold / warm average ratio: ${ratio.toFixed(2)}x`);
}

main().catch((error) => {
  console.error(
    `benchmark-prover-starts failed: ${error instanceof Error ? error.message : String(error)}`
  );
  process.exitCode = 1;
});
