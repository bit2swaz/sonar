# Sonar

Sonar is a Solana-native ZK coprocessor prototype built around an Anchor program, an off-chain proving pipeline, and a thin developer surface for submitting requests and registering verifiers. The repo now contains a full request -> prove -> callback -> index loop, a real CPI SDK, a developer CLI for verifier registration, Criterion benchmarks for hot paths, and CI/security automation suitable for active development.

Sonar is not production-ready yet. The current state is best described as a hardened devnet-quality system with one end-to-end vertical slice (`historical_avg`) and the core primitives needed to expand toward a multi-computation coprocessor.

## Current status

- On-chain program supports `register_verifier`, `request`, `callback`, and `refund`.
- Off-chain services include a coordinator, prover, indexer, and Geyser plugin.
- `historical_avg` runs end-to-end with an ignored integration test and CI coverage.
- `crates/sdk` provides a real Anchor CPI helper for downstream programs.
- `crates/cli` provides `sonar-cli register` for verifier registration.
- CI runs Rust checks, Anchor build/tests, dependency/license scanning, and secret scanning.
- Benchmarks exist for coordinator and prover hot paths.

## Architecture

```mermaid
flowchart LR
    A[Client or CPI caller] -->|request| B[Sonar Anchor program]
    B -->|stores RequestMetadata + ResultAccount PDAs| C[(Solana state)]
    B -->|emits request and inputs logs| D[Coordinator listener]
    D -->|fetch account history when needed| E[Indexer HTTP API]
    D -->|enqueue ProverJob| F[(Redis)]
    F --> G[Prover]
    G -->|proof + result| F
    F --> H[Coordinator callback worker]
    H -->|callback| B
    B -->|verify Groth16 + write result + invoke callback| I[Consumer callback program]
    B -->|fee payout| J[Prover authority]
    K[Geyser plugin] --> L[(Postgres)]
    L --> E
```

## Repository map

| Path                              | Purpose                                                                                |
| --------------------------------- | -------------------------------------------------------------------------------------- |
| `program/`                        | Anchor program that owns request/result/verifier state and verifies proofs on callback |
| `crates/coordinator/`             | Log listener, Redis dispatcher, and callback submission worker                         |
| `crates/prover/`                  | SP1 execution, Groth16 wrapping, artifact export, computation registry                 |
| `crates/indexer/`                 | Geyser plugin, Postgres persistence, and Axum HTTP API                                 |
| `crates/sdk/`                     | Anchor CPI helper for downstream Sonar consumers                                       |
| `crates/cli/`                     | `sonar-cli` for verifier registration                                                  |
| `programs/historical_avg_client/` | Example consumer program that requests the historical-average computation              |
| `echo_callback/`                  | Minimal callback target used by integration flows                                      |
| `tests/`                          | Rust integration/e2e coverage, including historical-average orchestration              |
| `docs/`                           | Current-state, target-state, roadmap, architecture, and contribution docs              |

## Prerequisites

- Rust stable
- Node.js 20+
- Solana CLI 3.0.13
- Anchor CLI 0.32.1
- Docker (for integration and end-to-end flows)
- PostgreSQL and Redis when running the off-chain stack outside the test harness

## Quick start

```bash
npm install
cargo test --workspace -- --skip integration
anchor build
anchor test
```

For the full historical-average vertical slice:

```bash
cargo build --bins
cargo build -p sonar-indexer --lib
anchor build
cargo test --test e2e_historical_avg -- --ignored --nocapture
```

## Common workflows

### Rust quality gates

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --skip integration
```

### Benchmarks

```bash
cargo bench -p sonar-coordinator
cargo bench -p sonar-prover
```

### Export verifier artifacts

```bash
cargo run --bin sonar-export-artifacts -- artifacts
```

### Register a verifier

```bash
cargo run -p sonar-cli -- register \
  --elf-path programs/historical_avg/elf/riscv32im-succinct-zkvm-elf \
  --keypair ~/.config/solana/id.json \
  --rpc-url "$SOLANA_RPC_URL"
```

`sonar-cli` hashes the ELF to derive the `computation_id`, resolves a Groth16 verifier artifact, builds `register_verifier`, and submits the transaction to Solana.

## Configuration

- `config/default.toml` and `config/devnet.toml` define runtime configuration.
- The off-chain stack is environment-driven for secrets and endpoints.
- The indexer expects Postgres, the coordinator/prover expect Redis, and Solana RPC/WS endpoints are supplied via config or env vars.

## Validation and security automation

- CI: format, clippy, unit/integration tests, Anchor build/tests, e2e flow, and demo verification.
- Security workflow: `cargo audit`, `cargo deny`, and `gitleaks`.
- Pre-commit hooks: Rust fmt/clippy, `cargo deny`, `cargo audit`, and Prettier for Markdown/JSON/YAML.

## Limits of the current repo

- Production economics, fee policy, and capacity planning are still evolving.
- Verifier registration exists, but operational lifecycle management and governance are still manual.
- The repo proves one strong vertical slice today rather than a broad catalog of production computations.
- The system is still oriented around devnet/local-validator workflows, not a hardened mainnet rollout.

## Read next

- `docs/SSOT.md` for the current implementation truth
- `docs/ARCHITECTURE.md` for component and lifecycle detail
- `docs/ROADMAP.md` for what is done versus what remains
- `docs/PROD_TARGET.md` for the desired production shape
- `docs/CONTRIBUTING.md` for local workflow expectations
- `SECURITY.md` for disclosure and secure-development guidance# Sonar

Sonar is a Solana ZK coprocessor that lets an on-chain program request off-chain computation, queue the work, generate a proof, and return a verified result through a callback transaction.

## what problem it solves

Solana programs are fast, but they are constrained by compute limits, account access rules, and transaction size. sonar splits the workflow into two parts:

- the on-chain program stores a request, escrows the fee, and verifies a Groth16 proof
- off-chain services watch for requests, prepare inputs, run the computation, and submit the callback

this lets the repository model larger jobs such as indexed historical queries without forcing all work into a single Solana transaction.

## current status

the repository already includes:

- a working Anchor program with `request`, `callback`, and `refund`
- a PostgreSQL-backed indexer and an HTTP query endpoint
- a Redis-backed coordinator and prover pipeline
- two registered prover computations: `fibonacci` and `historical_avg`
- a full TypeScript integration suite for the on-chain demo verifier path
- a checked-in Rust historical-average e2e test and an automated demo verification script

the repository does not yet include a production-final historical-average verifier path on-chain. the program now recognizes `HISTORICAL_AVG_COMPUTATION_ID`, the coordinator forwards prover-produced `public_inputs`, and the repository includes local end-to-end coverage for the historical-average flow, but the on-chain historical-average verification path is still an MVP-specific implementation rather than a finished production verifier story.

## high-level architecture

```mermaid
flowchart LR
    client[Client Program / Test Harness]
    sonar[Sonar Anchor Program]
    coord[Coordinator]
    redis[(Redis)]
    prover[Prover]
    indexer[Indexer]
    pg[(PostgreSQL)]

    client -->|Request instruction| sonar
    sonar -->|Request logs & PDA state| coord
    coord -->|Historical queries| indexer
    indexer --> pg

    coord -->|ProverJob JSON| redis
    redis -->|Job consumed| prover
    prover -->|ProverResponse JSON| redis
    redis -->|Response consumed| coord

    coord -->|Callback transaction| sonar
```

## repository layout

- `program/` - Anchor on-chain program
- `echo_callback/` - test-only callback helper program
- `crates/common/` - shared config, metrics, and types
- `crates/indexer/` - Geyser plugin, PostgreSQL access, and HTTP server
- `crates/coordinator/` - log listener, Redis dispatcher, and callback worker
- `crates/prover/` - computation registry, SP1 wrappers, and Redis worker
- `bin/` - executable entry points for indexer, coordinator, and prover
- `programs/` - SP1 guest programs and committed ELF files
- `docs/` - current-state docs, production-target docs, roadmap, and contribution docs

important docs:

- [docs/SSOT.md](docs/SSOT.md) - factual current implementation status
- [docs/PROD_TARGET.md](docs/PROD_TARGET.md) - forward-looking production architecture target
- [docs/ROADMAP.md](docs/ROADMAP.md) - canonical transition plan from current MVP to the production target

## key features

- on-chain request escrow and callback verification
- PDA-based request and result storage
- Redis queue handoff between coordinator and prover
- PostgreSQL account-history storage with SQLx migrations
- axum query endpoint for historical lamport balances
- SP1 guest execution for fibonacci and historical average templates
- mock-prover support for local development
- ci coverage for fmt, clippy, tests, audit, deny, Anchor build, and Anchor tests

## quick start

### 1. clone and install tools

```bash
git clone git@github.com:bit2swaz/sonar.git
cd sonar
rustup toolchain install 1.94.1 --component rustfmt clippy rust-src
npm install
```

install the Solana and Anchor versions used in the repository:

```bash
sh -c "$(curl -sSfL https://release.anza.xyz/v3.0.13/install)"
cargo install anchor-cli --version 0.32.1 --locked
```

### 2. start local dependencies

start Redis:

```bash
redis-server
```

start PostgreSQL with Docker:

```bash
docker run --rm -it \
	--name sonar-postgres \
	-e POSTGRES_PASSWORD=postgres \
	-e POSTGRES_DB=sonar \
	-p 5432:5432 \
	postgres:16-alpine
```

### 3. export required environment variables

```bash
export SOLANA_RPC_URL=http://127.0.0.1:8899
export SOLANA_WS_URL=ws://127.0.0.1:8900
export HELIUS_API_KEY=dummy
export HELIUS_RPC_URL=http://127.0.0.1:8899
export DATABASE_URL=postgresql://postgres:postgres@localhost:5432/sonar
export REDIS_URL=redis://127.0.0.1:6379
export SP1_PROVING_KEY=/tmp/sp1.key
export GROTH16_PARAMS=/tmp/groth16.params
```

### 4. build the workspace

```bash
cargo build --workspace
anchor build
```

to generate deterministic prover metadata artifacts for the currently registered computations:

```bash
cargo run --bin sonar-export-artifacts
```

if `anchor build` fails because of the Solana platform-tools cargo version, use the helper script instead:

```bash
./scripts/build-program.sh
```

### 5. run tests

```bash
cargo test --workspace -- --skip integration
solana-test-validator --quiet &
anchor test
```

note: Anchor CLI `0.32.1` can emit a trailing validator-cleanup `os error 2` even when the suite passes. in CI we tolerate that specific cleanup quirk and instead fail only on actual failing specs or a missing success banner.

### 5.1 run benchmarks and profiles

Criterion benchmarks are checked in for coordinator/prover hot paths:

```bash
cargo bench -p sonar-coordinator --bench coordinator_hot_paths
cargo bench -p sonar-prover --bench prover_hot_paths
```

to list available benchmark ids or capture a Criterion profile window:

```bash
cargo bench -p sonar-coordinator --bench coordinator_hot_paths -- --list
cargo bench -p sonar-prover --bench prover_hot_paths -- --profile-time=5
```

for system-level flamegraphs, install `cargo-flamegraph` and `perf`, then run one bench target at a time:

```bash
cargo install flamegraph
cargo flamegraph -p sonar-coordinator --bench coordinator_hot_paths
cargo flamegraph -p sonar-prover --bench prover_hot_paths
```

### 6. run the services

indexer:

```bash
SONAR_CONFIG=config/default.toml cargo run --bin sonar-indexer
```

prover:

```bash
SONAR_CONFIG=config/default.toml cargo run --bin sonar-prover
```

coordinator:

```bash
SONAR_CONFIG_PATH=config/default.toml cargo run --bin sonar-coordinator
```

### 7. deploy to devnet

the checked-in `Anchor.toml` already contains a devnet program id for `sonar`. for a fresh deployment under your own authority, sync keys first and then deploy:

```bash
solana config set --url devnet
anchor keys sync
anchor build
anchor deploy --provider.cluster devnet
```

notes:

- local tests also use the `echo_callback` helper program
- `Anchor.toml` only defines a devnet entry for `sonar`, not for `echo_callback`
- `config/devnet.toml` is older than the current `Config` struct and is missing `indexer.http_port` and `coordinator.indexer_url`
- the demo scripts use safer high local ports by default: Postgres `15432`, Redis `16379`, indexer `18080`, RPC `18899`, faucet `19900`

## configuration reference

the runtime config is loaded by `sonar_common::config::Config` from a TOML file with `${ENV_VAR}` expansion.

| key                                    | type     | used by                | meaning                                           |
| -------------------------------------- | -------- | ---------------------- | ------------------------------------------------- |
| `network.rpc_url`                      | `string` | coordinator            | Solana HTTP RPC endpoint                          |
| `network.ws_url`                       | `string` | coordinator            | Solana websocket endpoint                         |
| `network.chain_id`                     | `string` | shared                 | network label                                     |
| `strategy.min_profit_floor_usd`        | `f64`    | shared                 | reserved strategy setting                         |
| `strategy.gas_buffer_multiplier`       | `f64`    | shared                 | reserved strategy setting                         |
| `strategy.max_gas_price_gwei`          | `f64`    | shared                 | reserved strategy setting                         |
| `rpc.helius_api_key`                   | `string` | shared                 | reserved external RPC setting                     |
| `rpc.helius_rpc_url`                   | `string` | shared                 | reserved external RPC setting                     |
| `indexer.geyser_plugin_path`           | `string` | indexer                | expected path to the built plugin library         |
| `indexer.database_url`                 | `string` | indexer                | PostgreSQL connection string                      |
| `indexer.concurrency`                  | `usize`  | indexer                | configured worker concurrency value               |
| `indexer.http_port`                    | `u16`    | indexer                | axum server listen port                           |
| `coordinator.redis_url`                | `string` | coordinator and prover | Redis connection string                           |
| `coordinator.callback_timeout_seconds` | `u64`    | shared config          | callback timeout setting                          |
| `coordinator.max_concurrent_jobs`      | `usize`  | prover                 | semaphore limit for prover jobs                   |
| `coordinator.indexer_url`              | `string` | coordinator            | base URL for the indexer HTTP API                 |
| `prover.sp1_proving_key_path`          | `string` | prover                 | configured proving-key path                       |
| `prover.groth16_params_path`           | `string` | prover                 | configured Groth16 params path                    |
| `prover.mock_prover`                   | `bool`   | prover                 | sets mock proving mode when `SP1_PROVER` is unset |
| `observability.log_level`              | `string` | shared                 | tracing filter level                              |
| `observability.metrics_port`           | `u16`    | shared                 | metrics server port                               |

environment variables used directly by binaries:

- `SONAR_CONFIG` - used by `sonar-indexer` and `sonar-prover`
- `SONAR_CONFIG_PATH` - used by `sonar-coordinator`
- `SONAR_COORDINATOR_KEYPAIR_PATH` - optional signer for callback submissions
- `SP1_PROVER` - optional override for prover mode

## how to run tests

Rust checks:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --skip integration
```

security and dependency checks:

```bash
cargo audit
cargo deny check
```

Anchor tests:

```bash
solana-test-validator --quiet &
anchor build
anchor test
```

avoid `anchor test --skip-build` here. the full test flow needs Anchor's normal build-and-deploy sequence so the local validator has the workspace programs loaded.

Historical-average end-to-end flow:

```bash
SP1_PROVER=mock cargo test --test e2e_historical_avg -- --ignored --nocapture

./scripts/verify-demo.sh
```

## limitations

the current repository has a few important gaps that matter when you evaluate readiness:

- the historical-average path is still an MVP verification flow rather than a finished production verifier design
- the historical-average on-chain path currently uses a dedicated MVP verification helper rather than a fully separate production verifying-key rollout
- `crates/sdk` is still a stub
- `tests/integration.rs` and `tests/property.rs` are placeholders for later phases

## contributing

see [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md).

## license

this project is licensed under the MIT license. see [LICENSE](LICENSE).
