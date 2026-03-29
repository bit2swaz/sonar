# Sonar – Solana ZK Coprocessor  
**ROADMAP (Full TDD, Production‑Ready)**

## How To Use This File

This roadmap is the complete execution plan for building Sonar end‑to‑end — from an empty directory to a production‑grade, fully‑tested ZK coprocessor running on Solana.

Every mini‑phase contains:
- A clear goal and strict definition of done
- A complete, self‑contained prompt to paste into Claude Sonnet (or another AI) in your AI IDE
- The prompt includes all context the LLM needs — no additional input required
- Every prompt includes a directive to **use Context7** to fetch the latest documentation for relevant libraries (e.g., Anchor, SP1, groth16‑solana)

**The TDD contract:**
Every mini‑phase writes tests BEFORE or ALONGSIDE implementation. No mini‑phase is complete until `cargo test` passes with zero failures and zero ignored tests (except fork tests that require a live Solana RPC, which are explicitly marked).

**Rules:**
1. Complete mini‑phases in strict order
2. `cargo build`, `cargo clippy -- -D warnings`, and `cargo test` must all pass before marking any mini‑phase complete
3. SSOT.md is the source of truth — if ROADMAP and SSOT conflict, SSOT wins
4. Keep both SSOT.md and ROADMAP.md in the repository root at all times
5. If a prompt produces broken code, paste the compiler error back and fix before moving on — never carry broken code forward

---

## Full Phase Overview

```
Phase 0  — Project Hygiene        (CI, git hooks, linting, security baseline)
Phase 1  — Foundation             (workspace, types, config, observability)
Phase 2  — Solana Program         (Anchor verifier, request/callback/refund, groth16 verification)
Phase 3  — Off‑Chain Prover       (SP1 integration, simple program, Groth16 wrapping)
Phase 4  — State Indexer          (Geyser plugin, PostgreSQL, account history)
Phase 5  — Coordinator & Queue    (Rust service, Redis, request dispatching)
Phase 6  — End‑to‑End MVP         (Historical average template, full flow on devnet)
Phase 7  — Testing & Hardening    (property tests, chaos tests, fork tests)
Phase 8  — Performance Optimisation (benchmarking, flamegraphs, recursion)
Phase 9  — Developer Experience   (Rust SDK, TS SDK, CLI, documentation)
Phase 10 — Testnet & Mainnet Readiness (deploy to devnet/testnet, grant applications)
Phase 11 — Token & Staking        ($SONAR token, staking program, slashing)
Phase 12 — Decentralisation & Cross‑Chain (permissionless provers, recursion, Wormhole)
Phase 13 — Open Source & Grants   (comprehensive docs, blog series, grant proposals)
```

---

# PHASE 0 — Project Hygiene

**Goal:** Establish the non‑negotiable production baseline before writing a single line of business logic. CI must be green. Linting must be strict. Secrets must never enter the repo. Security scanning must be automated. This phase costs almost nothing to set up and saves you from catastrophic mistakes later.

---

## Mini‑Phase 0.1 — Git, CI, and Linting Baseline

**Status: ✅ Complete**

**Definition of done:**
- `.github/workflows/ci.yml` runs on every push and PR — ✅ confirmed (`on: push/pull_request`, `branches: [main]`)
- CI runs: `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, `cargo audit`, `cargo deny check` — ✅ confirmed
- Pre‑commit hook in `scripts/pre-commit`, installer in `scripts/install-hooks.sh`, `.pre-commit-config.yaml` present — ✅ confirmed
- `.gitignore` and `.env.example` exist — no real secrets ever enter the repo — ✅ confirmed
- `cargo clippy --workspace -- -D warnings` passes — ✅ confirmed

---

**PROMPT 0.1**

```
You are building `sonar`, a production‑grade ZK coprocessor for Solana in Rust.
This is Mini‑Phase 0.1: Git, CI, and Linting Baseline.

Read SSOT.md in full before writing any code.

Your task is to set up all project hygiene infrastructure. Write every file below
completely — no stubs, no TODOs.

**Use Context7** to get the latest best‑practice patterns for GitHub Actions workflows with Rust, and for pre‑commit hook configurations. Ensure you use the most up‑to‑date versions of actions and tools.

File 1: `.github/workflows/ci.yml`
A GitHub Actions workflow that triggers on push and pull_request to main.
Jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable with components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --check
      - run: cargo clippy -- -D warnings
      - run: cargo test --workspace
      - run: cargo audit
        (install: cargo install cargo-audit first via cache)
    env:
      SOLANA_RPC_URL: "https://api.devnet.solana.com"
      SONAR_PROGRAM_ID: "5B1rXQ71oEWUPc3AemCBTQtb5pmGAnX1jbGvZKcgBy84"
      PRIVATE_KEY: "0x0000000000000000000000000000000000000000000000000000000000000000"

File 2: `.github/workflows/security.yml`
A separate workflow that runs weekly and on push to main:
  - cargo audit (known vulnerability check)
  - cargo deny check (license compliance + duplicate dependency check)

File 3: `.gitignore`
Comprehensive gitignore for Rust + Solana + secrets:
  /target
  /program/target
  /program/idl
  /program/out
  /indexer/target
  /coordinator/target
  *.env
  .env
  .env.*
  !.env.example
  logs/
  flamegraph.svg
  perf.data
  *.log
  /sonar-devnet/

File 4: `.env.example`
Template showing every required environment variable with placeholder values:
  SOLANA_RPC_URL=https://api.devnet.solana.com
  SONAR_PROGRAM_ID=5B1rXQ71oEWUPc3AemCBTQtb5pmGAnX1jbGvZKcgBy84
  PRIVATE_KEY=0x0000000000000000000000000000000000000000000000000000000000000000
  REDIS_URL=redis://localhost:6379
  DATABASE_URL=postgresql://postgres:password@localhost:5432/sonar
  SP1_PROVING_KEY=path/to/key
  HELIUS_API_KEY=YOUR_HELIUS_KEY_HERE

File 5: `deny.toml`
cargo‑deny config:
  [licenses]
  allow = ["MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "BSD-2-Clause",
           "BSD-3-Clause", "ISC", "Unicode-DFS-2016", "CC0-1.0", "Zlib"]
  [bans]
  multiple-versions = "warn"
  [advisories]
  vulnerability = "deny"
  unmaintained = "warn"
  yanked = "deny"

File 6: `.rustfmt.toml`
  edition = "2021"
  max_width = 100
  use_field_init_shorthand = true
  use_try_shorthand = true
  imports_granularity = "Crate"
  group_imports = "StdExternalCrate"

File 7: `scripts/pre-commit`
A shell script to install as a git pre‑commit hook:
  #!/bin/sh
  set -e
  cargo fmt --check
  cargo clippy -- -D warnings
  cargo test --workspace --quiet
  echo "Pre-commit checks passed."

File 8: `scripts/install-hooks.sh`
  #!/bin/sh
  cp scripts/pre-commit .git/hooks/pre-commit
  chmod +x .git/hooks/pre-commit
  echo "Git hooks installed."

Also create an empty `Cargo.toml` at root with just:
  [workspace]
  members = []
  resolver = "2"

...so that cargo commands work from day one before the full workspace is populated.

Write every file completely. No placeholders.
```

---

## Mini‑Phase 0.2 — Secret Scanning and Dependency Pinning

**Status: ✅ Complete**

**Definition of done:**
- `gitleaks` config (`.gitleaks.toml`) prevents accidental private key commits; `gitleaks-action@v2` in `security.yml` — ✅ confirmed
- `Cargo.lock` is committed (binary — always lock deps); `cargo metadata --format-version 1 --locked` step enforces this in CI — ✅ confirmed
- `rust-toolchain.toml` pins the toolchain channel to `stable` with `rustfmt`, `clippy`, `rust-src` components — ✅ confirmed
- `SECURITY.md` documents vulnerability reporting and known risk areas — ✅ confirmed
- `deny.toml` configured with allowed licenses and advisory policy — ✅ confirmed

---

**PROMPT 0.2**

```
You are building `sonar`. Read SSOT.md in full before writing any code.

This is Mini‑Phase 0.2: Secret Scanning and Dependency Pinning.

**Use Context7** to get the latest configuration for gitleaks (rules for Solana private keys and API keys). Also get the latest stable Rust version to pin.

File 1: `.gitleaks.toml`
Configure gitleaks to detect Solana private keys and API keys:
  title = "sonar gitleaks config"

  [[rules]]
  id = "solana-private-key"
  description = "Solana private key (base58)"
  regex = '''[1-9A-HJ-NP-Za-km-z]{88}'''
  tags = ["key", "solana"]

  [[rules]]
  id = "helius-api-key"
  description = "Helius API key"
  regex = '''[a-zA-Z0-9]{32,}'''
  tags = ["key", "helius"]
  [rules.allowlist]
  paths = [".env.example", "ROADMAP.md", "SSOT.md", "docs/"]

Add gitleaks step to `.github/workflows/security.yml`:
  - uses: gitleaks/gitleaks-action@v2
    env:
      GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}

File 2: `rust-toolchain.toml`
  [toolchain]
  channel = "1.94.1"
  components = ["rustfmt", "clippy", "rust-src"]
  targets = ["x86_64-unknown-linux-gnu"]

File 3: Update `.github/workflows/ci.yml`
Add a job that verifies Cargo.lock is committed and up to date:
  - run: cargo update --locked
    (this fails if Cargo.lock is stale, ensuring deps are always pinned)

File 4: `SECURITY.md`
  # Security Policy
  ## Reporting Vulnerabilities
  Do not open a public GitHub issue for security vulnerabilities.
  Email: [your email]
  
  ## Known Risk Areas
  - PRIVATE_KEY env var: never log, never commit, rotate immediately if exposed
  - RPC endpoints: treat API keys as secrets
  - Smart contract: Sonar program is not audited — use at your own risk
  - ZK proofs: verify all proofs on‑chain; never trust off‑chain provers

Write all files completely.
```

---

# PHASE 1 — Foundation

**Goal:** Build the complete Rust workspace with shared types, config, and observability. Every type has tests. Config loading is tested with property‑based inputs. Observability metrics are verified to register and increment correctly.

---

## Mini‑Phase 1.1 — Workspace Scaffold

**Status: ✅ Complete**

**Definition of done:**
- `cargo build --workspace` passes with zero errors — ✅ confirmed
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes with zero warnings — ✅ confirmed
- Workspace structure matches SSOT.md: `crates/{common,indexer,coordinator,prover,sdk,solana-program-shim}`, `bin/`, `config/`, `tests/`, `program/` — ✅ confirmed
- All crates compile; workspace resolver `"2"`, shared `[workspace.dependencies]` with pinned versions (anchor-lang 0.32.1, groth16-solana 0.2.0, sqlx 0.8, etc.) — ✅ confirmed

---

**PROMPT 1.1**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 1.1: Workspace Scaffold.

Create the complete Rust workspace. Write every file needed for
`cargo build --workspace` and `cargo clippy --workspace -- -D warnings`
to pass with zero errors and zero warnings.

Full repository structure from SSOT.md:
sonar/
├── program/                        (Anchor project)
│   ├── Cargo.toml
│   ├── src/lib.rs
│   └── tests/
├── crates/
│   ├── common/src/                 (shared types, config, metrics)
│   │   ├── lib.rs
│   │   ├── types.rs
│   │   ├── config.rs
│   │   ├── metrics.rs
│   │   └── pnl.rs
│   ├── indexer/src/                (Geyser plugin + PostgreSQL)
│   │   ├── lib.rs
│   │   ├── geyser_plugin.rs
│   │   └── db.rs
│   ├── coordinator/src/            (Rust service, Redis queue)
│   │   ├── lib.rs
│   │   ├── listener.rs
│   │   ├── dispatcher.rs
│   │   └── callback.rs
│   ├── prover/src/                 (SP1 + Groth16)
│   │   ├── lib.rs
│   │   ├── sp1_wrapper.rs
│   │   └── groth16_wrapper.rs
│   └── sdk/src/                    (Rust SDK for developers)
│       ├── lib.rs
│       └── macro.rs
├── bin/
│   ├── coordinator.rs              (binary entrypoint)
│   ├── indexer.rs                  (binary entrypoint)
│   └── prover.rs                   (binary entrypoint)
├── config/
│   ├── default.toml
│   └── devnet.toml
├── tests/
│   ├── integration/
│   │   └── mod.rs
│   └── property/
│       └── mod.rs
├── Cargo.toml                      (workspace root)
└── README.md

Root Cargo.toml — workspace with shared deps:
  [workspace]
  members = [
    "crates/common",
    "crates/indexer",
    "crates/coordinator",
    "crates/prover",
    "crates/sdk",
    "program",
  ]
  resolver = "2"

  [[bin]]
  name = "sonar-coordinator"
  path = "bin/coordinator.rs"

  [[bin]]
  name = "sonar-indexer"
  path = "bin/indexer.rs"

  [[bin]]
  name = "sonar-prover"
  path = "bin/prover.rs"

  [workspace.dependencies]
  # Async runtime
  tokio = { version = "1", features = ["full"] }
  # Solana
  solana-sdk = "1.18"
  solana-client = "1.18"
  solana-program = "1.18"
  anchor-lang = "0.30"
  # ZK
  sp1-sdk = "2.0"
  groth16-solana = "0.1"
  # Database
  sqlx = { version = "0.7", features = ["runtime-tokio-native-tls", "postgres", "uuid"] }
  # Redis
  redis = { version = "0.24", features = ["tokio-comp"] }
  # Serialization
  serde = { version = "1", features = ["derive"] }
  serde_json = "1"
  toml = "0.8"
  # Logging & metrics
  tracing = "0.1"
  tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
  prometheus = "0.13"
  # Errors
  anyhow = "1"
  thiserror = "1"
  # Testing
  proptest = "1"
  tokio-test = "0.4"
  mockall = "0.13"

Individual crate Cargo.toml requirements:
- Each must have [package] name, version="0.1.0", edition="2021"
- All deps use { workspace = true }
- common: serde, toml, anyhow, thiserror, tracing, prometheus, serde_json
- indexer: tokio, solana-sdk, sqlx, tracing, anyhow; dep on common
- coordinator: tokio, redis, solana-client, anyhow; dep on common
- prover: sp1-sdk, groth16-solana, anyhow; dep on common
- sdk: anchor-lang, solana-program, anyhow; dep on common
- program: anchor-lang, solana-program, anyhow; plus dependencies from program/Cargo.toml

Every .rs file must compile. Use empty pub mod declarations.
Use #[allow(dead_code, unused_imports)] at crate level temporarily.
Write every single file completely. Do not skip any.
```

---

## Mini‑Phase 1.2 — Shared Types with Full Test Coverage

**Status: ✅ Complete**

**Definition of done:**
- All types in `crates/common/src/types.rs` compile with correct derives (`Debug`, `Clone`, `PartialEq`, `Serialize`, `Deserialize`) — ✅ confirmed
- Every type has construction, serde round-trip, and equality tests; `ProofVerificationResult` has `is_success()`/`result()` method tests; `GasEstimate` has the arithmetic property test — ✅ confirmed
- `cargo test -p sonar-common` passes with **34 tests, 0 failures** — ✅ confirmed

---

**PROMPT 1.2**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 1.2: Shared Types with Full Test Coverage.

Write `crates/common/src/types.rs` with all shared types AND their complete
test suite. Tests are written in the same file in a #[cfg(test)] module.

Types to implement (from SSOT Appendix A and other sections):

1. RequestMetadata (matching Anchor account struct)
   - request_id: [u8; 32]
   - callback_program: Pubkey
   - result_account: Pubkey
   - deadline: u64
   - fee: u64
   - status: RequestStatus
   - completed_at: Option<u64>
   - bump: u8
   - Derives: Debug, Clone, PartialEq, Serialize, Deserialize

2. RequestStatus enum: Pending, Completed, Refunded
   Derives: Debug, Clone, PartialEq, Serialize, Deserialize

3. RequestParams (instruction data)
   - request_id: [u8; 32]
   - computation_id: [u8; 32]
   - inputs: Vec<u8>
   - deadline: u64
   - fee: u64
   Derives: Debug, Clone, PartialEq, Serialize, Deserialize

4. CallbackParams
   - proof: Vec<u8>
   - public_inputs: Vec<Vec<u8>>
   - result: Vec<u8>
   Derives: Debug, Clone, PartialEq, Serialize, Deserialize

5. ComputationId: [u8; 32] (hash of SP1 program binary)

6. ComputationRequest
   - id: ComputationId
   - inputs: Vec<u8>
   - deadline: u64
   - fee: u64
   Derives: Debug, Clone, PartialEq, Serialize, Deserialize

7. ComputationResult
   - request_id: [u8; 32]
   - result: Vec<u8>
   - proof: Vec<u8>
   - timestamp: u64
   Derives: Debug, Clone, PartialEq, Serialize, Deserialize

8. ProofVerificationResult enum
   - Success { result: Vec<u8> }
   - Failure { reason: String }
   Derives: Debug, Clone, PartialEq
   Implement: fn is_success(&self) -> bool, fn result(&self) -> Option<&[u8]>

9. ProverJob
   - request_id: [u8; 32]
   - computation_id: ComputationId
   - inputs: Vec<u8>
   - deadline: u64
   - fee: u64
   - callback_program: Pubkey
   - result_account: Pubkey
   Derives: Debug, Clone, Serialize, Deserialize

10. ProverResponse
    - request_id: [u8; 32]
    - result: Vec<u8>
    - proof: Vec<u8>
    - gas_used: u64  (simulated, for metrics)
    Derives: Debug, Clone, Serialize, Deserialize

11. GasEstimate
    - cu_units: u64
    - cu_price: u64
    - total_cost_lamports: u64
    Derives: Debug, Clone, Serialize, Deserialize

Tests to write in #[cfg(test)] mod tests:

// RequestMetadata tests
test_request_metadata_construction — build a RequestMetadata, assert all fields
test_request_metadata_serde_roundtrip — serialize to JSON, deserialize back, assert eq

// RequestStatus tests
test_request_status_serde — serialize Pending, assert string "pending"

// RequestParams tests
test_request_params_serde_roundtrip

// CallbackParams tests
test_callback_params_serde_roundtrip

// ComputationResult tests
test_computation_result_serde_roundtrip

// ProofVerificationResult tests
test_verification_success_is_success — Success variant, assert is_success() true
test_verification_failure_is_not_success — Failure variant, assert is_success() false
test_verification_success_result — Success with result, assert result() == Some(...)

// ProverJob & ProverResponse tests
test_prover_job_serde_roundtrip
test_prover_response_serde_roundtrip

// GasEstimate tests
test_gas_estimate_total — cu_units * cu_price == total_cost_lamports

Update common/src/lib.rs:
  pub mod types;
  pub mod config;  // stub for now
  pub mod metrics; // stub for now
  pub mod pnl;     // stub for now
  pub use types::*;

Write the complete file with all types and all tests. Every test must pass.
```

---

## Mini‑Phase 1.3 — Config System with Env Var Expansion Tests

**Status: ✅ Complete**

**Definition of done:**
- Config loads correctly from TOML + `${VAR}` env var expansion via `Config::load_str` / `Config::load` — ✅ confirmed
- Missing required env var returns a clear `Err("Missing env var: VAR_NAME")` — ✅ confirmed (`test_expand_env_vars_missing_var` passes)
- Invalid TOML returns a clear error — ✅ confirmed (`test_load_str_invalid_toml` passes)
- All edge cases tested: single substitution, multiple substitutions, no vars, missing var, full TOML load with mocked env — ✅ confirmed (6 config tests)
- `cargo test -p sonar-common` passes — ✅ confirmed (34 tests total, 0 failures)

---

**PROMPT 1.3**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 1.3: Config System with Env Var Expansion Tests.

Write `crates/common/src/config.rs` and `config/default.toml` with full tests.

Config structs (must exactly mirror SSOT.md TOML structure):

  #[derive(Debug, Clone, Deserialize)]
  pub struct Config {
      pub network: NetworkConfig,
      pub strategy: StrategyConfig,
      pub rpc: RpcConfig,
      pub indexer: IndexerConfig,
      pub coordinator: CoordinatorConfig,
      pub prover: ProverConfig,
      pub observability: ObservabilityConfig,
  }

  NetworkConfig: rpc_url: String, ws_url: String, chain_id: String
  StrategyConfig: min_profit_floor_usd: f64, gas_buffer_multiplier: f64, max_gas_price_gwei: f64
  RpcConfig: helius_api_key: String, helius_rpc_url: String
  IndexerConfig: geyser_plugin_path: String, database_url: String, concurrency: usize
  CoordinatorConfig: redis_url: String, callback_timeout_seconds: u64, max_concurrent_jobs: usize
  ProverConfig: sp1_proving_key_path: String, groth16_params_path: String, mock_prover: bool
  ObservabilityConfig: log_level: String, metrics_port: u16

Implement:
  impl Config {
      pub fn load(path: &str) -> anyhow::Result<Self>
      // Reads TOML, expands ${VAR_NAME} patterns with env vars
      // Returns Err if file not found, invalid TOML, or missing env var

      pub fn load_str(toml_str: &str) -> anyhow::Result<Self>
      // Same but from a string — used in tests

      fn expand_env_vars(input: &str) -> anyhow::Result<String>
      // Replaces all ${VAR_NAME} with env var values
      // Returns Err("Missing env var: VAR_NAME") if any var is unset
  }

Write `config/default.toml` with all required fields, referencing environment variables where appropriate (e.g., database_url = "${DATABASE_URL}").

Write `config/devnet.toml`:
  chain_id = "devnet"
  rpc_url = "${SOLANA_RPC_URL}"
  ws_url = "${SOLANA_WS_URL}"
  min_profit_floor_usd = 0.01
  max_gas_price_gwei = 1.0
  All other fields same as default.toml

Tests in #[cfg(test)] mod tests:

test_load_str_valid_config — inline TOML with env vars pre‑substituted,
  assert chain_id = "mainnet", etc.

test_expand_env_vars_substitutes_correctly — set env var TEST_VAR=hello,
  input "${TEST_VAR}", assert output "hello"

test_expand_env_vars_multiple — two vars in one string, both substituted

test_expand_env_vars_missing_var — unset var, assert Err contains var name

test_expand_env_vars_no_vars — plain string, returned unchanged

test_load_str_invalid_toml — garbage input, assert Err

test_default_toml_loads_with_env — set all required env vars (mock),
  load config/default.toml, assert fields match

Write all files completely. Every test must pass.
```

---

## Mini‑Phase 1.4 — Observability: Metrics and Tracing

**Status: ✅ Complete**

**Definition of done:**
- All Prometheus metrics (`requests_submitted`, `proofs_verified`, `proofs_failed`, `total_fees_earned_lamports`, `prover_utilization`, `request_latency_seconds`, `verification_cu_used`, `active_provers`) register without panicking — ✅ confirmed
- Metrics increment correctly; `render()` output verified in tests — ✅ confirmed (`test_counter_increments`, `test_failed_counter_with_label`, `test_histogram_record`, `test_gauge_set` all pass)
- `test_independent_registries` confirms two `Metrics::new()` instances do not share state — ✅ confirmed
- `crates/common/src/tracing_init.rs` implements `init_tracing(log_level)` — ✅ confirmed
- `cargo test -p sonar-common` passes — ✅ confirmed (34 tests, 0 failures)

---

**PROMPT 1.4**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 1.4: Observability: Metrics and Tracing.

Write `crates/common/src/metrics.rs` with complete tests.

Implement the Metrics struct tracking all key metrics (aligned with SSOT):

  pub struct Metrics {
      pub requests_submitted: IntCounter,
      pub proofs_verified: IntCounter,
      pub proofs_failed: IntCounterVec,  // with "reason" label
      pub total_fees_earned_lamports: Counter,
      pub prover_utilization: Gauge,
      pub request_latency_seconds: Histogram,
      pub verification_cu_used: Histogram,
      pub active_provers: Gauge,
  }

  impl Metrics {
      pub fn new() -> anyhow::Result<Self>
      // Register all metrics with a fresh Registry

      pub fn registry(&self) -> &Registry

      pub fn render(&self) -> String
      // Returns prometheus text format

      pub async fn start_server(registry: Registry, port: u16) -> anyhow::Result<()>
      // Starts tokio TCP listener on 0.0.0.0:port, GET /metrics returns render()
  }

Also write `crates/common/src/tracing_init.rs`:
  pub fn init_tracing(log_level: &str)
  // Initialises tracing‑subscriber with env‑filter
  // Uses JSON format in release, pretty format in debug

Tests in #[cfg(test)]:

test_metrics_new_registers_all — create Metrics::new(), assert render() contains all metrics

test_counter_increments — increment requests_submitted 3 times, render, assert value = 3

test_failed_counter_with_label — increment proofs_failed with reason="InvalidProof" twice,
  assert rendered value with that label = 2

test_histogram_record — record request_latency_seconds with value 1.5, render, assert present

test_gauge_set — set prover_utilization to 0.75, render, assert present

test_independent_registries — two Metrics::new() instances do not share state

test_metrics_server_responds — start server on port 19090, send HTTP GET /metrics,
  assert 200 response with body containing metric names

Write all files completely. All tests must pass.
```

---

# PHASE 2 — Solana Program

**Goal:** Write, test with full TDD, and deploy the Sonar verifier program. Every instruction path through the program has corresponding tests on a real Solana devnet fork. No path is untested.

---

## Mini‑Phase 2.1 — Anchor Project Setup

**Status: ✅ Complete**

**Definition of done:**
- `anchor build` passes with zero errors — ✅ confirmed
- Placeholder Anchor test script passes — ✅ confirmed (`sonar integration checks passed` printed); the post-suite non-zero exit from `anchor test` is traced to Anchor CLI v0.32 validator-process cleanup (ENOENT in its own signal handler), not the test body. CI captures the output and verifies the success message explicitly.
- Root `Anchor.toml` correctly configures the workspace program for local validator testing — ✅ confirmed (`cluster = "Localnet"`, `startup_wait = 15000`, root `Anchor.toml`)

---

**PROMPT 2.1**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 2.1: Anchor Project Setup.

Set up the complete Anchor project for the workspace-root Anchor configuration and the `program/` crate.

**Use Context7** to get the latest Anchor framework setup instructions and best practices for Solana program testing.

File 1: program/Cargo.toml
  [package]
  name = "sonar-program"
  version = "0.1.0"
  edition = "2021"

  [lib]
  crate-type = ["cdylib", "lib"]
  name = "sonar_program"

  [features]
  default = []
  no-entrypoint = []
  test-bpf = []

  [dependencies]
  anchor-lang = { workspace = true }
  anchor-spl = "0.30"
  solana-program = { workspace = true }
  groth16-solana = { workspace = true }
  # ... other deps

File 2: Anchor.toml
  [programs.localnet]
  sonar = "5B1rXQ71oEWUPc3AemCBTQtb5pmGAnX1jbGvZKcgBy84"

  [programs.devnet]
  sonar = "5B1rXQ71oEWUPc3AemCBTQtb5pmGAnX1jbGvZKcgBy84"

  [registry]
  url = "https://api.apr.dev"

  [provider]
  cluster = "Localnet"
  wallet = "~/.config/solana/id.json"

  [scripts]
  test = "TS_NODE_PROJECT=./tsconfig.json node -r ts-node/register/transpile-only ./program/tests/sonar.ts"

  [workspace]
  members = ["program"]

  [toolchain]
  anchor_version = "0.32.1"
  solana_version = "3.0.13"
  package_manager = "npm"

  [test]
  startup_wait = 15000

File 3: program/src/lib.rs
Placeholder:
  use anchor_lang::prelude::*;
  declare_id!("5B1rXQ71oEWUPc3AemCBTQtb5pmGAnX1jbGvZKcgBy84");

  #[program]
  pub mod sonar {
      use super::*;
      pub fn request(ctx: Context<Request>, params: RequestParams) -> Result<()> {
          Ok(())
      }
      pub fn callback(ctx: Context<Callback>, params: CallbackParams) -> Result<()> {
          Ok(())
      }
      pub fn refund(ctx: Context<Refund>) -> Result<()> {
          Ok(())
      }
  }

  #[derive(Accounts)]
  pub struct Request<'info> {
      // fields to be filled later
  }
  #[derive(Accounts)]
  pub struct Callback<'info> {
      // ...
  }
  #[derive(Accounts)]
  pub struct Refund<'info> {
      // ...
  }

  #[derive(AnchorSerialize, AnchorDeserialize, Clone)]
  pub struct RequestParams {
      // fields
  }
  // etc.

File 4: program/tests/sonar.ts
Placeholder test that just verifies the program can be deployed.

CI / Ubuntu note:
  - GitHub's `ubuntu-latest` runner needs `pkg-config` and `libudev-dev` installed before `cargo install anchor-cli --version 0.32.1 --locked`, otherwise the transitive `hidapi` build fails while looking for `libudev`.

Write all files. `anchor build` and `anchor test` must pass (with mock RPC).
```

---

## Mini‑Phase 2.2 — Full Implementation of Sonar Program

**Status: ✅ Complete**

**Definition of done:**
- Program implements `request`, `callback`, `refund` instructions per SSOT Appendix A — ✅ confirmed
- Uses `groth16-solana` syscall for on-chain Groth16 verification via `alt_bn128` — ✅ confirmed
- All accounts and constraints correctly implemented (PDA seeds, `has_one`, deadline, fee, status guards) — ✅ confirmed
- `anchor build` passes — ✅ confirmed; `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo test --workspace` also pass
- Verifier registry seeded with `DEMO_COMPUTATION_ID` (sha256 of `sonar:demo-groth16-fixture:v1`) and the `groth16-solana` crate's built-in fixture verifying key — ✅ confirmed
- Result PDA (`[b"result", request_id]`), callback CPI with `sonar_callback` discriminator, fee transfer, and all 18 error codes implemented — ✅ confirmed

---

**PROMPT 2.2**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 2.2: Full Implementation of Sonar Program.

Write the complete `program/src/lib.rs` with all instructions, account structs, and validation logic, exactly as specified in SSOT Appendix A.

**Use Context7** to get the latest Anchor patterns for PDA derivation, CPI to callback programs, and using the groth16‑solana crate for verification.

Key requirements:
- Program ID must match the one in Anchor.toml.
- `request` instruction:
  - Derives PDA for `request_metadata` using `[b"request", request_id]`.
  - Initializes both `request_metadata` and a Sonar-owned `result_account` PDA using `[b"result", request_id]`.
  - Stores `payer`, `callback_program`, `result_account`, `computation_id`, deadline, fee, and status in metadata.
  - Transfers fee from payer to the request_metadata account (or holds in vault). Use token program if fee is in $SONAR, or native lamports if in SOL.
- `callback` instruction:
  - Uses `has_one = result_account` and `has_one = callback_program` to ensure correctness.
  - Verifies proofs through `groth16-solana` using a compile-time verifier registry keyed by `computation_id`.
  - On success: writes result to the `result_account` PDA, updates metadata status to Completed, CPIs into the callback program using a `sonar_callback` payload, and transfers fee to prover (or vault).
  - On failure: returns `ErrorCode::ProofVerificationFailed`.
- `refund` instruction:
  - Checks deadline has passed, status is Pending, and the signer matches the original payer.
  - Transfers fee back to payer, updates status to Refunded.
- All error codes defined.

Implement verification via `groth16-solana`, which wraps the Solana alt_bn128 verification syscalls. Seed the verifier registry with a built-in demo computation ID and verifying key so Phase 2.3 can use the crate’s known-good fixtures.

Write the complete code. No stubs.
```

---

## Mini‑Phase 2.3 — Program Tests: Full TDD on Solana Devnet Fork

**Status:** ✅ Complete

**Definition of done:**
- All instructions tested with real transactions on a forked devnet (using `solana-test-validator` and `anchor test`)
- Access control: 3 tests
- Request flow: 3 tests
- Callback flow: 5 tests (including proof verification)
- Refund flow: 2 tests
- Edge cases: 4 tests
- Total: minimum 17 tests, all passing

**Delivered:**
- `program/tests/sonar.ts` now covers 17 end-to-end Anchor integration tests across access control, request flow, callback flow, refund flow, and edge cases.
- A minimal `echo_callback` helper program is included so callback CPI behavior is exercised with a real executable target.
- CI verifies the passing test-body sentinel (`sonar integration checks passed`) to tolerate the known Anchor v0.32 validator cleanup ENOENT after a successful suite.

---

**PROMPT 2.3**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 2.3: Program Tests — Full TDD on Solana Devnet Fork.

Write `program/tests/sonar.ts` (TypeScript) with comprehensive tests using Anchor's testing framework. Use a local validator (solana-test-validator) and forked devnet state.

**Use Context7** to get the latest Anchor testing patterns, including `program.provider.connection`, `Program.provider.wallet`, and how to generate valid Groth16 proofs for testing (or mock the syscall).

Write these tests:

ACCESS CONTROL (3 tests):
  test_only_original_payer_can_refund – a different signer attempts `refund`, should fail with `RefundPayerMismatch`.
  test_request_accepts_executable_callback_program – request succeeds when callback program is executable.
  test_request_rejects_non_executable_callback_program – request fails when callback program is not executable.

REQUEST FLOW (3 tests):
  test_request_creates_metadata – call request, fetch the PDA, verify fields match.
  test_request_transfers_fee – ensure fee lamports are moved from payer to program (or held).
  test_request_with_invalid_input – e.g., deadline in the past, should revert.

CALLBACK FLOW (5 tests):
  test_callback_with_valid_proof – mock a valid Groth16 proof (can be dummy with known verification key), call callback, verify result_account gets written and status updated.
  test_callback_with_invalid_proof – invalid proof should fail with ProofVerificationFailed.
  test_callback_wrong_request_id – use wrong request_id, should fail with InvalidRequestId.
  test_callback_after_deadline – deadline passed, should revert (or allow? According to SSOT, callback can still be submitted? Actually deadline is for request expiry, callback may still be accepted if within deadline; after deadline, refund only. So test that callback after deadline reverts.)
  test_callback_pays_prover – verify that after successful callback, prover receives fee (if not burned).

REFUND FLOW (2 tests):
  test_refund_before_deadline – attempt refund before deadline, should revert.
  test_refund_after_deadline – after deadline, refund succeeds and fee returned to payer.

EDGE CASES (4 tests):
  test_callback_with_duplicate_request_id – callback for already completed request, should revert.
  test_callback_with_wrong_result_pda – passing a mismatched result PDA should fail (`has_one` / seeds constraint).
  test_large_inputs – ensure program can handle inputs up to 10 KiB (or config limit).
  test_concurrent_requests – multiple requests in one block, all should succeed.

Write all tests using Anchor's `it` blocks, with proper `await` and assertions.
Use `program.provider.connection` to fetch accounts after transactions.
For proof verification, you may need to generate a dummy Groth16 proof. Since we don't have a real prover yet, we can use a mock verification key and proof that the syscall will accept. For initial tests, you can use a known valid proof from groth16‑solana's test fixtures. Or, we can bypass verification with a feature flag for testing (but not ideal). Instead, we can use `solana_program_test` with a mocked syscall. However, for simplicity, we can generate a valid Groth16 proof using a trivial circuit (e.g., prove that 1+1=2) and embed it in the test. Use the `groth16-solana` crate's example to generate such a proof.

Write the complete test file. All tests must pass.
```

---

# PHASE 3 — Off‑Chain Prover

**Goal:** Integrate SP1 to generate proofs for arbitrary Rust code, then wrap them in Groth16 for on‑chain verification. The prover service must be able to accept a job, execute the Rust program, generate a STARK proof, wrap it in Groth16, and return the proof + result.

---

## Mini‑Phase 3.1 — SP1 Setup and Simple Program

**Definition of done:**
- SP1 SDK is correctly configured in the prover crate
- A trivial SP1 program (e.g., `fibonacci`) compiles and generates a proof
- `cargo test -p sonar-prover` passes with a test that runs SP1 and verifies the proof

---

**PROMPT 3.1**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 3.1: SP1 Setup and Simple Program.

**Use Context7** to get the latest SP1 SDK documentation, including how to set up a SP1 program, generate a proof, and verify it.

Set up the prover crate:

File 1: `crates/prover/Cargo.toml`
  [package]
  name = "sonar-prover"
  version = "0.1.0"
  edition = "2021"

  [dependencies]
  sp1-sdk = { workspace = true }
  groth16-solana = { workspace = true }
  anyhow = { workspace = true }
  tracing = { workspace = true }
  serde = { workspace = true }
  sonar-common = { workspace = true }

  [features]
  mock = []

File 2: `crates/prover/src/sp1_wrapper.rs`
  - Define a function `fn build_sp1_program(elf_path: &str) -> anyhow::Result<Vec<u8>>` that loads the ELF.
  - Define `fn run_sp1_program(elf: &[u8], inputs: &[u8]) -> anyhow::Result<(Vec<u8>, Vec<u8>)>` that executes the program and returns (output, stark_proof).
  - Use SP1 SDK's `ProverClient` and `SP1Stdin`.

File 3: `crates/prover/src/groth16_wrapper.rs`
  - Define `fn wrap_stark_to_groth16(stark_proof: &[u8], public_inputs: &[Vec<u8>]) -> anyhow::Result<Vec<u8>>` that uses SP1's `sp1-groth16` to wrap.
  - This step is essential to get a proof that Solana can verify.

File 4: `crates/prover/src/lib.rs`
  - Expose a public API: `pub fn prove(computation_id: &[u8; 32], inputs: &[u8]) -> anyhow::Result<(Vec<u8>, Vec<u8>)>` that loads the correct ELF (based on computation_id), runs SP1, wraps to Groth16, and returns (proof, result).

Create a simple SP1 program (e.g., `fibonacci`) in a separate directory (e.g., `programs/fibonacci`). Provide instructions on how to compile it to ELF and include the ELF as bytes in the binary (or load from file). For testing, we can hardcode the ELF as a byte array.

Write tests:
  - `test_sp1_fibonacci` – run the program with input 10, verify output is 55, and that a proof is generated.
  - `test_groth16_wrapping` – take the SP1 proof, wrap it, and verify that the resulting proof is not empty.
  - `test_prove_end_to_end` – call `prove` with the computation ID of the fibonacci program, assert result correct.

All tests must pass. Use `#[cfg(feature = "mock")]` to skip actual proving in CI if needed, but for the MVP we can run them on a machine with SP1 installed.

Write all code completely.
```

---

## Mini‑Phase 3.2 — Prover Service (Rust binary)

**Definition of done:**
- Prover binary listens for jobs (e.g., from Redis), executes `prove`, and pushes results to a response queue
- Handles multiple concurrent jobs
- Graceful shutdown
- Tests with mock job queue

---

**PROMPT 3.2**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 3.2: Prover Service.

Write `bin/prover.rs` that:
  - Loads config.
  - Connects to Redis.
  - Subscribes to a queue (e.g., `sonar:jobs`).
  - For each job (serialized ProverJob), spawn a tokio task to:
      * Look up the ELF for the computation_id (from a registry, initially from a map).
      * Call `sonar_prover::prove` with the inputs.
      * On success, push a ProverResponse to `sonar:responses`.
      * On failure, log error and maybe push a failure response.
  - Limits concurrent tasks via `Semaphore`.
  - Handles SIGTERM by draining ongoing tasks and shutting down.

Write `crates/prover/src/registry.rs` to manage computation_id -> ELF path (or embed ELF as bytes). For MVP, use a static map: e.g., `["hist_avg"] -> ELF bytes`.

Write integration tests that mock Redis (use `redis::Mock` or a local Docker container in CI). Use `tokio::test` to spawn the prover binary as a separate process? Alternatively, test the `run_prover` function with a mock Redis connection.

All tests must pass.
```

---

# PHASE 4 — State Indexer

**Goal:** Build the Geyser plugin that streams account changes to PostgreSQL, and a query API for historical state.

---

## Mini‑Phase 4.1 — Geyser Plugin Skeleton

**Definition of done:**
- Geyser plugin compiles and logs account updates to console
- `cargo test -p sonar-indexer` passes (unit tests for plugin logic)

---

**PROMPT 4.1**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 4.1: Geyser Plugin Skeleton.

**Use Context7** to get the latest Solana Geyser plugin interface documentation.

Write `crates/indexer/src/geyser_plugin.rs`:
  - Implement the `geyser_plugin_interface::GeyserPlugin` trait.
  - In `on_load`, parse config.
  - In `on_account_update`, log account pubkey, lamports, data, etc. (for now).
  - Return success.

Write `crates/indexer/src/lib.rs` with the plugin entry point.

Add a simple test that creates a `GeyserPlugin` instance and calls `on_account_update` with dummy data, verifying it doesn't panic.

Write the plugin's `Cargo.toml` with necessary dependencies: `solana-geyser-plugin-interface`, `solana-sdk`, `log`, etc.

The plugin must be compilable to a shared library (cdylib). Ensure `crate-type = ["cdylib"]` in the crate's Cargo.toml.

Write the complete code.
```

---

## Mini‑Phase 4.2 — PostgreSQL Integration and Account History

**Definition of done:**
- Plugin writes account changes to PostgreSQL (using SQLx)
- Schema created automatically on first run
- Historical queries return correct account state at any slot
- Tests with a local PostgreSQL instance

---

**PROMPT 4.2**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 4.2: PostgreSQL Integration and Account History.

**Use Context7** to get the latest SQLx patterns for async PostgreSQL in Rust.

Extend `geyser_plugin.rs` to:
  - On `on_load`, create a connection pool to the database (using config).
  - Run migrations (embed SQL using `sqlx::migrate!`).
  - On `on_account_update`, insert a row into `account_history` (with slot, pubkey, lamports, owner, etc.) using a batch writer for performance.

Create a module `crates/indexer/src/db.rs` with:
  - A function `insert_account_batch` that accepts a slice of `AccountUpdate` and inserts using `COPY` or batched inserts.
  - A function `query_account_history(pubkey: &Pubkey, from_slot: u64, to_slot: u64) -> Vec<AccountState>` that queries the database.
  - A function `query_account_snapshot(pubkey: &Pubkey, slot: u64) -> Option<AccountState>` that gets the latest state at or before that slot.

Write SQL migrations to create tables:
  - `account_history` (as in SSOT Appendix D)
  - `slot_metadata`
  - `request_tracking` (optional for now)

Write tests using `sqlx::PgPool` with a temporary database (or Docker). Test that inserts and queries work correctly.

All tests must pass.
```

---

# PHASE 5 — Coordinator & Queue

**Goal:** Build the coordinator service that listens to Solana events (from the program), dispatches jobs to the prover queue, and handles callbacks.

---

## Mini‑Phase 5.1 — Event Listener and Job Dispatcher

**Definition of done:**
- Coordinator subscribes to Sonar program logs (using RPC subscription) to detect `request` events
- Converts events to ProverJob and pushes to Redis queue
- Tests with mock RPC and Redis

---

**PROMPT 5.1**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 5.1: Event Listener and Job Dispatcher.

**Use Context7** to get the latest Solana RPC subscription patterns in Rust (`solana-client`).

Write `crates/coordinator/src/listener.rs`:
  - Connects to Solana RPC WebSocket.
  - Subscribes to program logs for the Sonar program ID.
  - Parses logs to extract `request` events (look for `"Program log: Request"` and the request ID).
  - For each event, constructs a ProverJob (fetching additional data from RPC if needed) and sends it to a Redis queue.

Write `crates/coordinator/src/dispatcher.rs`:
  - A task that consumes from a Redis queue (`sonar:jobs`) and pushes to a separate queue for the prover? Actually, the listener already pushes to `sonar:jobs`, which the prover consumes. So this is just the listener part.

Write `crates/coordinator/src/callback.rs`:
  - A task that listens for responses from the prover (Redis `sonar:responses`).
  - For each response, constructs a callback transaction (using the `callback` instruction) and submits it to the Solana network.
  - Should handle retries and timeouts.

Write `bin/coordinator.rs` that spawns these tasks.

Write integration tests using `redis::Mock` and `solana_client::mock` to simulate events. Verify that a ProverJob is created and enqueued, and that callback transactions are submitted.

All tests must pass.
```

---

# PHASE 6 — End‑to‑End MVP

**Goal:** Tie everything together: a developer can write a simple Rust function (e.g., historical average), the indexer provides data, the prover generates a proof, and the coordinator returns it on‑chain.

---

## Mini‑Phase 6.1 — Historical Average Template

**Definition of done:**
- A Rust program (SP1) that takes a pubkey and slot range and computes the average lamports using historical data from the indexer.
- The program is compiled to ELF and registered in the prover.
- An example Solana program (e.g., `test_client`) demonstrates using the SDK to request this computation and receive the result.
- Full flow on devnet: request → prover job → callback → result verified.

---

**PROMPT 6.1**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 6.1: Historical Average Template.

**Use Context7** to get SP1 SDK examples for reading external data (the indexer's database). Since SP1 cannot directly query a database, the input must be provided as part of the proof's public inputs. The indexer must gather the required data and include it in the job.

Design:
  1. The user's Solana program calls `sonar::request` with `ComputationId::HistoricalAvg`, parameters (pubkey, slots).
  2. The coordinator, upon seeing the request, queries the indexer for the account balances at each slot in the range.
  3. It creates a ProverJob containing the list of balances (as bytes) as inputs.
  4. The prover runs the SP1 program that takes the list and computes the average, outputs the result and a proof.
  5. The coordinator submits the callback with the proof and result.

Implement:
  - SP1 program in `programs/historical_avg` that takes `(Vec<u64>)` and returns `u64` (average). Use simple iteration.
  - In prover, add this computation ID to the registry.
  - In coordinator, when dispatching, fetch balances from the indexer (using its query API). This requires adding an HTTP endpoint to the indexer to serve queries.
  - Add a simple query server (e.g., using `axum`) to the indexer that provides `GET /account_history/:pubkey?from_slot=...&to_slot=...` returning JSON list of lamports.

Write all components and test the end‑to‑end flow on a local devnet with `solana-test-validator`. Use a test client program that requests the average and logs the result.

All tests must pass.
```

---

# PHASE 7 — Testing & Hardening

**Goal:** Property tests for all formulas, chaos tests for component failures, and fork tests against real Solana mainnet state.

---

## Mini‑Phase 7.1 — Property Tests for ZK and Math

**Definition of done:**
- `proptest` tests for the historical average algorithm (correctness under arbitrary inputs)
- Property tests for Groth16 verification edge cases
- `cargo test --workspace` passes

---

**PROMPT 7.1**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 7.1: Property Tests for ZK and Math.

Write `tests/property/historical_avg.rs` using `proptest`:
  - Generate random lists of u64 values (balances).
  - Compute the average using the same algorithm as the SP1 program.
  - Compare to a reference implementation.
  - Property: average should be within [min, max] of the list.
  - Property: sum of balances equals length * average.

Write `tests/property/groth16.rs`:
  - Use `proptest` to generate random Groth16 proofs (or invalid ones) and test that verification fails appropriately.

Write `tests/property/account_history_queries.rs`:
  - Generate random account updates, insert into test DB, and verify queries return correct results.

All property tests must run with 10,000 iterations and pass.
```

---

## Mini‑Phase 7.2 — Chaos Tests

**Definition of done:**
- Simulate Redis disconnection, RPC failures, and prover crashes
- Verify coordinator and prover recover without crashing
- All chaos tests pass

---

**PROMPT 7.2**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 7.2: Chaos Tests.

Write `tests/chaos/mod.rs` with scenarios:
  - Redis connection drops and reconnects – ensure prover/coordinator resume processing.
  - Solana RPC returns errors – coordinator retries with backoff.
  - Prover binary crashes – supervisor restarts it (test using process supervision).
  - Database becomes unavailable – indexer buffers updates and replays when DB recovers.

Implement mocks for Redis, RPC, and database to simulate failures. Use `tokio::time` to simulate delays.

All tests must pass without panics.
```

---

# PHASE 8 — Performance Optimisation

**Goal:** Benchmark critical paths, profile with flamegraphs, and implement recursion for batch verification.

---

## Mini‑Phase 8.1 — Benchmarks

**Definition of done:**
- `criterion` benchmarks for:
  - SP1 proving time for a sample program
  - Groth16 verification (on‑chain simulation)
  - Database queries (average over 1000 slots)
  - Redis job serialization/deserialization
- Baseline numbers recorded

---

**PROMPT 8.1**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 8.1: Benchmarks.

Add `benches/` directory with `criterion` benchmarks:
  - `bench_sp1_prove` – measures time to prove a simple SP1 program (e.g., fibonacci).
  - `bench_groth16_verify` – measures time to verify a Groth16 proof using `groth16-solana` (use a real proof).
  - `bench_indexer_query` – queries 1000 slots for a single account.
  - `bench_coordinator_dispatch` – measures end‑to‑end time from request event to job enqueued.

Write a `scripts/flamegraph.sh` to run `cargo flamegraph` on the prover binary while processing a job.

All benchmarks must run without errors.
```

---

# PHASE 9 — Developer Experience

**Goal:** Build the Rust SDK with macros, TypeScript SDK, CLI tool, and comprehensive documentation.

---

## Mini‑Phase 9.1 — Rust SDK with Macro

**Definition of done:**
- `#[sonar_compute]` macro that transforms a Rust function into a SP1 program and generates the necessary CPI code.
- Example usage compiles.
- Tests that the macro produces correct code.

---

**PROMPT 9.1**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 9.1: Rust SDK with Macro.

**Use Context7** to get the latest `proc_macro` patterns and `syn`/`quote` crate usage.

Write `crates/sdk/src/lib.rs`:
  - Define `#[sonar_compute]` attribute macro that:
      * Parses the annotated function (must be `pub fn` with parameters that are `Serialize`).
      * Generates a struct with the same parameters.
      * Implements `SonarCompute` trait for that struct.
      * Provides a `request` method that constructs the CPI to the Sonar program.

Write `crates/sdk/tests/test_macro.rs`:
  - A test that uses the macro to define a compute function and then invokes it, verifying that the generated code compiles.

Write example in `examples/simple.rs` that demonstrates using the macro.

All tests must pass.
```

---

# PHASE 10 — Testnet & Mainnet Readiness

**Goal:** Deploy to devnet/testnet, run smoke tests, and prepare grant applications.

---

## Mini‑Phase 10.1 — Deploy to Devnet/Testnet

**Definition of done:**
- Sonar program deployed to devnet and testnet
- Coordinator, indexer, prover running in a test environment
- Smoke test script passes (opportunities detected and verified)

---

**PROMPT 10.1**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 10.1: Deploy to Devnet/Testnet.

Create deployment scripts:
  - `scripts/deploy-devnet.sh` – uses `anchor deploy` to deploy the program to devnet, updates `config/devnet.toml` with the new program ID.
  - `scripts/deploy-testnet.sh` – similar for testnet.
  - `scripts/run-smoke.sh` – runs a test client that requests a simple computation, waits for callback, and verifies the result.

Write `docs/TESTNET_VALIDATION.md` with step‑by‑step instructions.

Create a `Makefile` with targets: `deploy-devnet`, `deploy-testnet`, `smoke-test`.

All scripts must be tested locally.
```

---

# PHASE 11 — Token & Staking (Phase 2+)

**Goal:** Introduce $SONAR token, staking program, slashing, and rewards distribution.

---

## Mini‑Phase 11.1 — Token Program and Staking

**Definition of done:**
- SPL token created (or a custom token) with the tokenomics defined in SSOT.
- Staking program that allows provers to stake tokens and receive rewards.
- Slashing logic implemented.
- All tests pass.

---

**PROMPT 11.1**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 11.1: Token Program and Staking.

**Use Context7** to get the latest SPL token and Anchor staking examples.

Write `programs/sonar-token` (or extend the main program) to include:
  - Instructions to stake $SONAR tokens for a prover.
  - A vault that holds fees.
  - On successful callback, transfer fees to the prover (or to treasury) and burn a portion.

Write a staking program that:
  - Allows provers to lock tokens.
  - Slashes stake for liveness misses or invalid proofs.
  - Distributes rewards proportionally.

Write tests for staking, slashing, and fee distribution.

All tests must pass.
```

---

# PHASE 12 — Decentralisation & Cross‑Chain

**Goal:** Make the prover set permissionless, implement recursive aggregation, and add Wormhole integration for cross‑chain proofs.

---

## Mini‑Phase 12.1 — Permissionless Prover Registration

**Definition of done:**
- Any user can stake tokens to become a prover.
- A leader election mechanism selects which prover processes each job.
- Proofs can be submitted by any prover (slashing if wrong).
- Tests for registration and election.

---

**PROMPT 12.1**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 12.1: Permissionless Prover Registration.

Extend the coordinator to accept proof submissions from any prover (instead of a fixed set). Add a registry on‑chain that tracks staked provers. Use a simple round‑robin or random assignment for job distribution.

Write tests that simulate multiple provers, slashing for invalid proofs, and reward distribution.

All tests must pass.
```

---

## Mini‑Phase 12.2 — Recursive Aggregation

**Definition of done:**
- Multiple SP1 proofs are aggregated into one using SP1 recursion.
- On‑chain verification of the aggregated proof is under 200k CU.
- Tests for batch processing.

---

**PROMPT 12.2**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 12.2: Recursive Aggregation.

**Use Context7** to get the latest SP1 recursion examples.

Implement a batcher that:
  - Collects pending responses from prover.
  - When a batch is ready, runs an SP1 program that recursively verifies all the individual STARK proofs and outputs a single STARK proof.
  - Wraps that in Groth16 and submits a single callback transaction.
  - Updates all request metadata accordingly.

Write tests that generate multiple requests, ensure they are batched and verified correctly.

All tests must pass.
```

---

# PHASE 13 — Open Source & Grants

**Goal:** Polish documentation, write blog posts, and submit grant applications to Solana Foundation, ESP, etc.

---

## Mini‑Phase 13.1 — Production Documentation

**Definition of done:**
- README.md is world‑class, with badges, architecture diagram, quick start, and links.
- ARCHITECTURE.md explains design decisions, data flow, and security model.
- CONTRIBUTING.md guides external contributors.
- All docs render correctly on GitHub.

---

**PROMPT 13.1**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 13.1: Production Documentation.

Write the following files:

File 1: `README.md`
  - Project title and description.
  - Badges: CI, license, Rust version, etc.
  - One‑line summary.
  - Why Sonar exists.
  - Architecture diagram (ASCII or link to diagram).
  - Features list.
  - Quick start (5 commands to get a simple example running).
  - Configuration reference.
  - Observability (metrics).
  - How to run tests.
  - Deployment guide (link to docs/).
  - Contributing (link to CONTRIBUTING.md).
  - License.

File 2: `docs/ARCHITECTURE.md`
  - Full data flow diagram.
  - Detailed explanation of each component: on‑chain program, indexer, coordinator, prover.
  - Security model (trust in ZK proofs only, slashing, etc.).
  - Performance considerations and target metrics.
  - Known limitations and future work.

File 3: `CONTRIBUTING.md`
  - Setup instructions.
  - Code style (rustfmt, clippy).
  - Test requirements.
  - PR process.

File 4: `CHANGELOG.md`
  - Initial version entries.

Write all files to the highest quality. Ensure the architecture diagram is clear.
```

---

## Mini‑Phase 13.2 — Blog Series and Grant Applications

**Definition of done:**
- Outline for a blog explaining the development of Sonar.
- Grant applications to Solana Foundation, EF ESP, etc. are written.

---

**PROMPT 13.2**

```
You are building `sonar`. Read SSOT.md in full before writing any code.
This is Mini‑Phase 13.2: Blog Series and Grant Applications.

Write the following files:

File 1: `docs/BLOG_SERIES.md`
Outline for 6 posts:
  Post 1: "Why Solana Needs a ZK Coprocessor"
  Post 2: "Building a Solana Program That Verifies Groth16 Proofs"
  Post 3: "SP1: The zkVM That Lets You Write Rust, Not Circuits"
  Post 4: "The Indexer: Storing Solana's Entire State History"
  Post 5: "Coordinator & Prover: The Off‑Chain Brain"
  Post 6: "From Zero to Decentralized: The Roadmap to Permissionless"

File 2: `docs/GRANTS.md`
Complete applications for:
  - Solana Foundation Grants Program
  - EF ESP (if applicable)
  - Helius / Light Protocol grants

Each application should include:
  - Project name and category.
  - Summary (100 words).
  - Problem statement.
  - Solution.
  - Technical approach.
  - Public goods value.
  - Team details.
  - Budget breakdown.
  - Success metrics.

Write all files completely. Make the grant applications compelling.
```

---

## Phase Completion Checklist

```
Phase 0  — Project Hygiene        [x] 0.1 [x] 0.2
Phase 1  — Foundation             [x] 1.1 [x] 1.2 [x] 1.3 [x] 1.4
Phase 2  — Solana Program         [x] 2.1 [x] 2.2 [x] 2.3
Phase 3  — Off‑Chain Prover       [ ] 3.1 [ ] 3.2
Phase 4  — State Indexer          [ ] 4.1 [ ] 4.2
Phase 5  — Coordinator & Queue    [ ] 5.1
Phase 6  — End‑to‑End MVP         [ ] 6.1
Phase 7  — Testing & Hardening    [ ] 7.1 [ ] 7.2
Phase 8  — Performance Optimisation [ ] 8.1
Phase 9  — Developer Experience   [ ] 9.1
Phase 10 — Testnet & Mainnet Readiness [ ] 10.1
Phase 11 — Token & Staking        [ ] 11.1
Phase 12 — Decentralisation & Cross‑Chain [ ] 12.1 [ ] 12.2
Phase 13 — Open Source & Grants   [ ] 13.1 [ ] 13.2
```

Total: 13 phases, 20+ mini‑phases, each with a self‑contained prompt.

---

**Rules For Using The Prompts**

1. **Always include both SSOT.md and ROADMAP.md in context.** The LLM needs both.

2. **Run prompts in strict phase order.** Dependencies are real — skipping phases will produce type errors and missing imports.

3. **The definition of done is a hard gate.** Do not proceed until:
   - `cargo build --workspace` passes
   - `cargo clippy --workspace -- -D warnings` passes
   - `cargo test --workspace` passes with zero failures
   - (For program phases) `anchor test` passes

4. **When a prompt produces broken code:** paste the exact compiler error back to the LLM as a follow‑up: “Fix this error: [error]”. Do not move on until fixed.

5. **Fork tests tagged `#[ignore]`:** These require a live RPC. Run them manually before deployment: `cargo test --workspace -- --ignored`.

6. **Keep SSOT.md updated** if architectural decisions change during implementation. The SSOT is not a historical document — it is the current truth.

7. **The blog series is not optional.** Writing about what you build is what gets you noticed. Do Phase 13 even if you think it’s not worth it. It is.