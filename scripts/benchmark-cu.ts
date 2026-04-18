import * as anchor from "@coral-xyz/anchor";
import type { Idl } from "@coral-xyz/anchor";
import {
  Connection,
  type Finality,
  Keypair,
  LAMPORTS_PER_SOL,
  PublicKey,
  SystemProgram,
  type Commitment,
  type ConfirmedSignatureInfo,
  type VersionedTransactionResponse,
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
const DEFAULT_PROGRAM_KEYPAIR_PATH = resolve(process.cwd(), "target/deploy/sonar_program-keypair.json");
const DEFAULT_CALLBACK_KEYPAIR_PATH = resolve(process.cwd(), "target/deploy/echo_callback-keypair.json");
const FALLBACK_PROGRAM_ID = new PublicKey("Gf7RSZYmfNJ5kv2AJvcv5rjCANP6ePExJR19D91MECLY");
const FALLBACK_CALLBACK_PROGRAM_ID = new PublicKey("J7jsJVQz6xbWFhyxRbzk7nH5ALhStztUNR1nPupnyjxS");
const DEFAULT_REQUEST_FEE_LAMPORTS = 100_000;
const DEFAULT_DEADLINE_SLOT_DELTA = 5_000;
const DEFAULT_TIMEOUT_SECONDS = 900;
const DEFAULT_POLL_INTERVAL_MS = 2_000;
const DEFAULT_SLOT_WINDOW = 128;
const DEFAULT_SIGNATURE_SCAN_LIMIT = 20;
const DEFAULT_COMMITMENT: Commitment = "confirmed";
const DEFAULT_FINALITY: Finality = "confirmed";
const DEFAULT_INSTRUCTION_BUDGET = 200_000;
const FIBONACCI_ELF_PATH = resolve(
  process.cwd(),
  "programs/fibonacci/elf/fibonacci-program"
);
const HISTORICAL_AVG_COMPUTATION_ID = Buffer.from([
  180, 134, 237, 198, 23, 219, 85, 143, 84, 245, 61, 62, 222, 122, 82, 179,
  3, 201, 204, 111, 144, 62, 32, 159, 91, 227, 160, 78, 252, 195, 98, 100,
]);

const PROGRAM_CONSUMED_REGEX_TEMPLATE = String.raw`Program {{PROGRAM_ID}} consumed ([\d,]+) of ([\d,]+) compute units`;

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
  signatureScanLimit: number;
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

type SubmittedRequest = {
  requestId: Buffer;
  requestMetadata: PublicKey;
  resultAccount: PublicKey;
  signature: string;
  confirmedSlot: number;
  wallClockMs: number;
};

type ResultAccountState = {
  isSet?: boolean;
  is_set?: boolean;
  writtenAt?: anchor.BN | null;
  written_at?: anchor.BN | null;
};

type ProgramConsumption = {
  consumed: number;
  budget: number;
  line: string;
};

type CallbackTransactionMatch = {
  signatureInfo: ConfirmedSignatureInfo;
  transaction: VersionedTransactionResponse;
};

function readProgramIdFromKeypair(path: string): PublicKey | null {
  if (!existsSync(path)) {
    return null;
  }

  try {
    const secretKey = Uint8Array.from(JSON.parse(readFileSync(path, "utf8")) as number[]);
    return Keypair.fromSecretKey(secretKey).publicKey;
  } catch {
    return null;
  }
}

const DEFAULT_PROGRAM_ID = readProgramIdFromKeypair(DEFAULT_PROGRAM_KEYPAIR_PATH) ?? FALLBACK_PROGRAM_ID;
const DEFAULT_CALLBACK_PROGRAM_ID = readProgramIdFromKeypair(DEFAULT_CALLBACK_KEYPAIR_PATH) ?? FALLBACK_CALLBACK_PROGRAM_ID;

function usage(): string {
  return [
    "Usage:",
    "  npx ts-node --transpile-only scripts/benchmark-cu.ts [options]",
    "",
    "Optional:",
    `  --rpc-url <url>                   Solana RPC URL (default: ${DEFAULT_RPC_URL})`,
    `  --wallet <path>                   Wallet keypair path (default: ${DEFAULT_WALLET_PATH})`,
    `  --program-id <pubkey>             Sonar program ID (default: ${DEFAULT_PROGRAM_ID.toBase58()})`,
    `  --callback-program <pubkey>       Callback program ID (default: ${DEFAULT_CALLBACK_PROGRAM_ID.toBase58()})`,
    `  --fee-lamports <lamports>         Request fee in lamports (default: ${DEFAULT_REQUEST_FEE_LAMPORTS})`,
    `  --deadline-slots <slots>          Slot delta added to the current slot (default: ${DEFAULT_DEADLINE_SLOT_DELTA})`,
    `  --timeout-seconds <seconds>       Max callback wait (default: ${DEFAULT_TIMEOUT_SECONDS})`,
    `  --poll-interval-ms <ms>           Poll interval for callback/signature discovery (default: ${DEFAULT_POLL_INTERVAL_MS})`,
    `  --signature-scan-limit <count>    Recent signatures to inspect on the result PDA (default: ${DEFAULT_SIGNATURE_SCAN_LIMIT})`,
    "  --computation <historical-avg|fibonacci>",
    "                                    Computation to request (default: historical-avg)",
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
    signatureScanLimit: DEFAULT_SIGNATURE_SCAN_LIMIT,
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
      case "--signature-scan-limit":
        if (!next) fail("--signature-scan-limit requires a value");
        options.signatureScanLimit = parsePositiveInteger(next, "--signature-scan-limit");
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
  const fromSlot = options.fromSlot ?? Math.max(0, toSlot - options.slotWindow);

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
      const isSet = Boolean(state.isSet ?? state.is_set ?? false);
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

async function fetchTransactionWithRetry(
  connection: Connection,
  signature: string,
  timeoutSeconds: number,
  pollIntervalMs: number
): Promise<VersionedTransactionResponse> {
  const deadlineAt = Date.now() + timeoutSeconds * 1000;

  while (Date.now() < deadlineAt) {
    const transaction = await connection.getTransaction(signature, {
      commitment: DEFAULT_FINALITY,
      maxSupportedTransactionVersion: 0,
    });
    if (transaction !== null) {
      return transaction;
    }
    await sleep(pollIntervalMs);
  }

  throw new Error(`timed out waiting for transaction details for ${signature}`);
}

async function submitRequest(
  program: anchor.Program<Sonar>,
  provider: anchor.AnchorProvider,
  options: CliOptions,
  resolvedInputs: ResolvedInputs
): Promise<SubmittedRequest> {
  const requestId = randomBytes(32);
  const requestMetadata = requestPda(options.programId, requestId);
  const resultAccount = resultPda(options.programId, requestId);
  const startedAt = performance.now();
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

  const confirmation = await provider.connection.confirmTransaction(
    {
      signature,
      blockhash: latestBlockhash.blockhash,
      lastValidBlockHeight: latestBlockhash.lastValidBlockHeight,
    },
    DEFAULT_COMMITMENT
  );

  if (confirmation.value.err) {
    throw new Error(
      `request transaction ${signature} failed: ${JSON.stringify(confirmation.value.err)}`
    );
  }

  return {
    requestId,
    requestMetadata,
    resultAccount,
    signature,
    confirmedSlot: confirmation.context.slot,
    wallClockMs: Math.round(performance.now() - startedAt),
  };
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function parseProgramConsumption(
  logMessages: readonly string[] | null | undefined,
  programId: PublicKey
): ProgramConsumption | null {
  if (!logMessages || logMessages.length === 0) {
    return null;
  }

  const pattern = PROGRAM_CONSUMED_REGEX_TEMPLATE.replace(
    "{{PROGRAM_ID}}",
    escapeRegExp(programId.toBase58())
  );
  const matcher = new RegExp(pattern);
  let matched: ProgramConsumption | null = null;

  for (const line of logMessages) {
    const result = matcher.exec(line);
    if (!result) {
      continue;
    }

    matched = {
      consumed: Number.parseInt(result[1].split(",").join(""), 10),
      budget: Number.parseInt(result[2].split(",").join(""), 10),
      line,
    };
  }

  return matched;
}

function isCallbackTransaction(
  transaction: VersionedTransactionResponse,
  programId: PublicKey
): boolean {
  const logMessages = transaction.meta?.logMessages ?? [];
  return (
    logMessages.some((line) => line.includes("Instruction: Callback")) &&
    parseProgramConsumption(logMessages, programId) !== null
  );
}

async function findCallbackTransaction(
  connection: Connection,
  requestSignature: string,
  resultAccount: PublicKey,
  programId: PublicKey,
  timeoutSeconds: number,
  pollIntervalMs: number,
  signatureScanLimit: number,
  minSlot: number
): Promise<CallbackTransactionMatch> {
  const deadlineAt = Date.now() + timeoutSeconds * 1000;
  const seenSignatures = new Set<string>();

  while (Date.now() < deadlineAt) {
    const signatures = await connection.getSignaturesForAddress(
      resultAccount,
      { limit: signatureScanLimit },
      DEFAULT_FINALITY
    );

    for (const signatureInfo of signatures) {
      if (signatureInfo.signature === requestSignature || signatureInfo.slot < minSlot) {
        continue;
      }
      if (seenSignatures.has(signatureInfo.signature)) {
        continue;
      }
      seenSignatures.add(signatureInfo.signature);

      const transaction = await connection.getTransaction(signatureInfo.signature, {
        commitment: DEFAULT_FINALITY,
        maxSupportedTransactionVersion: 0,
      });
      if (transaction === null || transaction.meta?.err) {
        continue;
      }
      if (!isCallbackTransaction(transaction, programId)) {
        continue;
      }

      return { signatureInfo, transaction };
    }

    await sleep(pollIntervalMs);
  }

  throw new Error(
    `timed out waiting ${timeoutSeconds}s to locate a callback transaction touching ${resultAccount.toBase58()}`
  );
}

function formatCu(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
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
  console.log(`Callback program: ${options.callbackProgramId.toBase58()}`);
  console.log(`Payer: ${provider.wallet.publicKey.toBase58()}`);
  console.log(
    `Payer balance: ${(payerBalanceLamports / LAMPORTS_PER_SOL).toFixed(4)} SOL (${payerBalanceLamports} lamports)`
  );
  console.log(`Computation: ${resolvedInputs.computationLabel}`);
  console.log(`Request fee: ${options.feeLamports} lamports`);
  console.log(`Deadline slot delta: ${options.deadlineSlotDelta}`);
  console.log(`Timeout: ${options.timeoutSeconds}s`);
  console.log("Submitting one devnet request and waiting for the real callback verification transaction...");

  const request = await submitRequest(program, provider, options, resolvedInputs);

  console.log(`Request signature: ${request.signature}`);
  console.log(`Request metadata PDA: ${request.requestMetadata.toBase58()}`);
  console.log(`Result PDA: ${request.resultAccount.toBase58()}`);
  console.log(`Request confirmed in ${request.wallClockMs}ms at slot ${request.confirmedSlot}`);

  await waitForCallbackCompletion(
    program,
    request.resultAccount,
    options.timeoutSeconds,
    options.pollIntervalMs,
    "benchmark-cu"
  );

  const callbackMatch = await findCallbackTransaction(
    provider.connection,
    request.signature,
    request.resultAccount,
    options.programId,
    options.timeoutSeconds,
    options.pollIntervalMs,
    options.signatureScanLimit,
    request.confirmedSlot
  );

  const callbackTransaction = callbackMatch.transaction ?? (await fetchTransactionWithRetry(
    provider.connection,
    callbackMatch.signatureInfo.signature,
    options.timeoutSeconds,
    options.pollIntervalMs
  ));
  const logMessages = callbackTransaction.meta?.logMessages ?? [];
  const sonarConsumption = parseProgramConsumption(logMessages, options.programId);
  if (sonarConsumption === null) {
    throw new Error(
      `unable to parse Sonar CU consumption from callback transaction ${callbackMatch.signatureInfo.signature}`
    );
  }

  const callbackProgramConsumption = parseProgramConsumption(
    logMessages,
    options.callbackProgramId
  );
  const transactionComputeUnits = callbackTransaction.meta?.computeUnitsConsumed ?? null;
  const budget = sonarConsumption.budget || DEFAULT_INSTRUCTION_BUDGET;
  const remainingForConsumerCallback = budget - sonarConsumption.consumed;
  const estimatedSonarCoreBeforeCallback = callbackProgramConsumption
    ? Math.max(sonarConsumption.consumed - callbackProgramConsumption.consumed, 0)
    : null;
  const estimatedCallbackHeadroom = estimatedSonarCoreBeforeCallback === null
    ? null
    : budget - estimatedSonarCoreBeforeCallback;

  console.log("");
  console.log("=== Sonar callback CU profile ===");
  console.log(`Callback signature: ${callbackMatch.signatureInfo.signature}`);
  console.log(`Callback slot: ${callbackTransaction.slot}`);
  if (transactionComputeUnits !== null && transactionComputeUnits !== undefined) {
    console.log(`Transaction-level compute units consumed: ${formatCu(transactionComputeUnits)}`);
  }
  console.log(`Sonar program consumed: ${formatCu(sonarConsumption.consumed)} / ${formatCu(budget)} CU`);
  console.log(`Remaining from ${formatCu(budget)} CU budget: ${formatCu(remainingForConsumerCallback)} CU`);
  if (callbackProgramConsumption !== null) {
    console.log(
      `Callback program consumed: ${formatCu(callbackProgramConsumption.consumed)} / ${formatCu(callbackProgramConsumption.budget)} CU`
    );
  }
  if (estimatedSonarCoreBeforeCallback !== null && estimatedCallbackHeadroom !== null) {
    console.log(
      `Estimated Sonar core before callback CPI: ${formatCu(estimatedSonarCoreBeforeCallback)} CU`
    );
    console.log(
      `Estimated callback headroom before Sonar's CPI: ${formatCu(estimatedCallbackHeadroom)} CU`
    );
  }
  console.log(`Matched Sonar log line: ${sonarConsumption.line}`);
}

main().catch((error) => {
  console.error(`benchmark-cu failed: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
});
