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
const DEFAULT_DURATION_SECONDS = 60;
const DEFAULT_COMMITMENT: Commitment = "confirmed";
const SUBMISSION_CONTEXT_TTL_MS = 10_000;
const FIBONACCI_ELF_PATH = resolve(
  process.cwd(),
  "programs/fibonacci/elf/fibonacci-program"
);

const DEMO_COMPUTATION_ID = Buffer.from([
  23, 199, 119, 83, 7, 207, 206, 48, 5, 163, 228, 138, 241, 216, 145, 91,
  193, 28, 25, 123, 203, 251, 9, 53, 2, 35, 72, 231, 68, 94, 197, 56,
]);

const HISTORICAL_AVG_COMPUTATION_ID = Buffer.from([
  180, 134, 237, 198, 23, 219, 85, 143, 84, 245, 61, 62, 222, 122, 82, 179,
  3, 201, 204, 111, 144, 62, 32, 159, 91, 227, 160, 78, 252, 195, 98, 100,
]);

type SupportedComputation = "demo" | "fibonacci" | "historical-avg";

type CliOptions = {
  durationSeconds: number;
  tps: number;
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
  fibonacciN: number;
};

type SubmitResult = {
  sequence: number;
  signature: string;
  elapsedMs: number;
};

type SubmissionContext = {
  blockhash: Awaited<ReturnType<Connection["getLatestBlockhash"]>>;
  currentSlot: number;
  fetchedAtMs: number;
};

type RunStats = {
  submitted: number;
  confirmed: number;
  failed: number;
  latenciesMs: number[];
};

function usage(): string {
  return [
    "Usage:",
    "  npx ts-node --transpile-only scripts/benchmark-throughput.ts --tps 20 [options]",
    "",
    "Required:",
    "  --tps <rate>                     Target sustained transactions per second",
    "",
    "Optional:",
    `  --duration <seconds>             Test duration in seconds (default: ${DEFAULT_DURATION_SECONDS})`,
    `  --rpc-url <url>                   Solana RPC URL (default: ${DEFAULT_RPC_URL})`,
    `  --wallet <path>                   Wallet keypair path (default: ${DEFAULT_WALLET_PATH})`,
    `  --program-id <pubkey>             Sonar program ID (default: ${DEFAULT_PROGRAM_ID.toBase58()})`,
    `  --callback-program <pubkey>       Callback program ID (default: ${DEFAULT_CALLBACK_PROGRAM_ID.toBase58()})`,
    `  --fee-lamports <lamports>         Request fee in lamports (default: ${DEFAULT_REQUEST_FEE_LAMPORTS})`,
    `  --deadline-slots <slots>          Slot delta added to the current slot (default: ${DEFAULT_DEADLINE_SLOT_DELTA})`,
    "  --computation <demo|fibonacci|historical-avg>",
    "                                    Computation to request (default: fibonacci)",
    "  --inputs-hex <hex>                Raw request inputs as hex bytes (demo mode override)",
    "  --observed-account <pubkey>       Historical-average input account",
    "  --from-slot <slot>                Historical-average input start slot",
    "  --to-slot <slot>                  Historical-average input end slot",
    "  --fib-n <n>                       Fibonacci input for the prover guest (default: 30)",
    "  --help                            Show this message",
  ].join("\n");
}

function fail(message: string): never {
  throw new Error(message);
}

function parsePositiveNumber(value: string, flagName: string): number {
  const parsed = Number.parseFloat(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    fail(`${flagName} must be a positive number, got \"${value}\"`);
  }
  return parsed;
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
    durationSeconds: DEFAULT_DURATION_SECONDS,
    tps: 0,
    rpcUrl: DEFAULT_RPC_URL,
    walletPath: DEFAULT_WALLET_PATH,
    programId: DEFAULT_PROGRAM_ID,
    callbackProgramId: DEFAULT_CALLBACK_PROGRAM_ID,
    feeLamports: DEFAULT_REQUEST_FEE_LAMPORTS,
    deadlineSlotDelta: DEFAULT_DEADLINE_SLOT_DELTA,
    computation: "fibonacci",
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
      case "--tps":
        if (!next) fail("--tps requires a value");
        options.tps = parsePositiveNumber(next, "--tps");
        index += 1;
        break;
      case "--duration":
        if (!next) fail("--duration requires a value");
        options.durationSeconds = parsePositiveInteger(next, "--duration");
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
        if (next !== "demo" && next !== "fibonacci" && next !== "historical-avg") {
          fail(
            `--computation must be one of: demo, fibonacci, historical-avg (got \"${next}\")`
          );
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
      case "--fib-n":
        if (!next) fail("--fib-n requires a value");
        options.fibonacciN = parsePositiveInteger(next, "--fib-n");
        index += 1;
        break;
      default:
        fail(`Unknown argument: ${arg}\n\n${usage()}`);
    }
  }

  if (options.tps === 0) {
    fail(`--tps is required\n\n${usage()}`);
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

  if (options.computation === "fibonacci" && !existsSync(FIBONACCI_ELF_PATH)) {
    fail(`Fibonacci ELF not found at ${FIBONACCI_ELF_PATH}`);
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

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function percentile(values: number[], fraction: number): number {
  if (values.length === 0) {
    return 0;
  }
  const sorted = [...values].sort((left, right) => left - right);
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * fraction) - 1));
  return sorted[index];
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

function resolveComputationInputs(options: CliOptions): { computationId: Buffer; inputs: Buffer } {
  if (options.computation === "fibonacci") {
    return {
      computationId: resolveFibonacciComputationId(),
      inputs: encodeFibonacciInputs(options.fibonacciN),
    };
  }

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

async function getSubmissionContext(
  connection: Connection,
  cachedContext?: SubmissionContext
): Promise<SubmissionContext> {
  const now = Date.now();
  if (cachedContext && now - cachedContext.fetchedAtMs < SUBMISSION_CONTEXT_TTL_MS) {
    return cachedContext;
  }

  const [blockhash, currentSlot] = await Promise.all([
    connection.getLatestBlockhash(DEFAULT_COMMITMENT),
    connection.getSlot(DEFAULT_COMMITMENT),
  ]);

  return {
    blockhash,
    currentSlot,
    fetchedAtMs: now,
  };
}

async function submitSingleRequest(
  sequence: number,
  program: anchor.Program<Sonar>,
  provider: anchor.AnchorProvider,
  options: CliOptions,
  submissionContext: SubmissionContext,
  computationId: Buffer,
  inputs: Buffer
): Promise<SubmitResult> {
  const requestId = randomBytes(32);
  const startedAt = performance.now();

  const transaction = await program.methods
    .request({
      requestId: Array.from(requestId),
      computationId: Array.from(computationId),
      inputs,
      deadline: new anchor.BN(submissionContext.currentSlot + options.deadlineSlotDelta),
      fee: new anchor.BN(options.feeLamports),
    })
    .accountsPartial({
      payer: provider.wallet.publicKey,
      callbackProgram: options.callbackProgramId,
    })
    .transaction();

  transaction.feePayer = provider.wallet.publicKey;
  transaction.recentBlockhash = submissionContext.blockhash.blockhash;

  const signedTransaction = await provider.wallet.signTransaction(transaction);
  const signature = await provider.connection.sendRawTransaction(signedTransaction.serialize(), {
    skipPreflight: true,
    maxRetries: 0,
    preflightCommitment: DEFAULT_COMMITMENT,
  });

  console.log(`[submit ${sequence}] ${signature}`);

  const confirmation = await provider.connection.confirmTransaction(
    {
      signature,
      blockhash: submissionContext.blockhash.blockhash,
      lastValidBlockHeight: submissionContext.blockhash.lastValidBlockHeight,
    },
    DEFAULT_COMMITMENT
  );

  if (confirmation.value.err) {
    throw new Error(
      `[confirm ${sequence}] ${signature} failed: ${JSON.stringify(confirmation.value.err)}`
    );
  }

  const elapsedMs = Math.round(performance.now() - startedAt);
  console.log(`[confirm ${sequence}] ${signature} in ${elapsedMs}ms`);

  return {
    sequence,
    signature,
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
  const startSlot = await provider.connection.getSlot(DEFAULT_COMMITMENT);
  const intervalMs = 1000 / options.tps;
  const runUntilMs = performance.now() + options.durationSeconds * 1000;
  const runStartedAt = performance.now();
  const pending = new Set<Promise<void>>();
  const stats: RunStats = {
    submitted: 0,
    confirmed: 0,
    failed: 0,
    latenciesMs: [],
  };

  let cachedContext: SubmissionContext | undefined;
  let sequence = 0;
  let nextDispatchAt = performance.now();

  console.log(`RPC URL: ${options.rpcUrl}`);
  console.log(`Program ID: ${options.programId.toBase58()}`);
  console.log(`Payer: ${provider.wallet.publicKey.toBase58()}`);
  console.log(
    `Payer balance: ${(payerBalanceLamports / LAMPORTS_PER_SOL).toFixed(4)} SOL (${payerBalanceLamports} lamports)`
  );
  console.log(`Callback program: ${options.callbackProgramId.toBase58()}`);
  console.log(`Computation: ${options.computation}`);
  console.log(`Target TPS: ${options.tps}`);
  console.log(`Duration: ${options.durationSeconds}s`);
  console.log(`Dispatch interval: ${intervalMs.toFixed(2)}ms`);
  console.log(`Start slot: ${startSlot}`);
  console.log(`Deadline slot delta: ${options.deadlineSlotDelta}`);
  console.log(`Request fee: ${options.feeLamports} lamports`);
  console.log(`Input bytes: ${inputs.length}`);
  console.log("Starting sustained throughput run...");

  while (performance.now() < runUntilMs) {
    const now = performance.now();
    const delayMs = nextDispatchAt - now;
    if (delayMs > 1) {
      await sleep(delayMs);
      continue;
    }

    nextDispatchAt += intervalMs;
    sequence += 1;
    stats.submitted += 1;

    cachedContext = await getSubmissionContext(provider.connection, cachedContext);

    const task = submitSingleRequest(
      sequence,
      program,
      provider,
      options,
      cachedContext,
      computationId,
      inputs
    )
      .then((result) => {
        stats.confirmed += 1;
        stats.latenciesMs.push(result.elapsedMs);
      })
      .catch((error: unknown) => {
        stats.failed += 1;
        console.error(`request ${sequence} failed: ${error instanceof Error ? error.message : String(error)}`);
      })
      .finally(() => {
        pending.delete(task);
      });

    pending.add(task);

    if (sequence % Math.max(1, Math.round(options.tps)) === 0) {
      const elapsedSeconds = (performance.now() - runStartedAt) / 1000;
      console.log(
        `[progress] t=${elapsedSeconds.toFixed(1)}s submitted=${stats.submitted} confirmed=${stats.confirmed} failed=${stats.failed} inFlight=${pending.size}`
      );
    }
  }

  await Promise.allSettled(Array.from(pending));

  const totalElapsedMs = Math.round(performance.now() - runStartedAt);
  const achievedTps = stats.confirmed / Math.max(1, totalElapsedMs / 1000);
  const p50 = Math.round(percentile(stats.latenciesMs, 0.5));
  const p95 = Math.round(percentile(stats.latenciesMs, 0.95));
  const p99 = Math.round(percentile(stats.latenciesMs, 0.99));

  console.log(`Finished sustained run in ${totalElapsedMs}ms.`);
  console.log(`Submitted: ${stats.submitted}`);
  console.log(`Confirmed: ${stats.confirmed}`);
  console.log(`Failed: ${stats.failed}`);
  console.log(`Achieved confirmed TPS: ${achievedTps.toFixed(2)}`);
  console.log(`Confirmation latency p50=${p50}ms p95=${p95}ms p99=${p99}ms`);
  console.log(
    "monitor the Redis queue depth and Postgres memory usage in the docker stats or Grafana."
  );

  if (stats.failed > 0) {
    process.exitCode = 1;
  }
}

main().catch((error) => {
  console.error(
    `benchmark-throughput failed: ${error instanceof Error ? error.message : String(error)}`
  );
  console.log(
    "monitor the Redis queue depth and Postgres memory usage in the docker stats or Grafana."
  );
  process.exitCode = 1;
});
