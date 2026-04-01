# Sonar

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

the repository does not yet include a fully verified historical-average callback path on-chain. the program verifier registry still only knows the built-in demo computation id, and the callback worker still sends empty `public_inputs`.

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
- `docs/` - architecture, roadmap, ssot, and contribution docs

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
sh -c "$(curl -sSfL https://release.anza.xyz/v2.3.13/install)"
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

if `anchor build` fails because of the Solana platform-tools cargo version, use the helper script instead:

```bash
./scripts/build-program.sh
```

### 5. run tests

```bash
cargo test --workspace -- --skip integration
solana-test-validator --quiet &
anchor test --skip-build
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

## configuration reference

the runtime config is loaded by `sonar_common::config::Config` from a TOML file with `${ENV_VAR}` expansion.

| key | type | used by | meaning |
| --- | --- | --- | --- |
| `network.rpc_url` | `string` | coordinator | Solana HTTP RPC endpoint |
| `network.ws_url` | `string` | coordinator | Solana websocket endpoint |
| `network.chain_id` | `string` | shared | network label |
| `strategy.min_profit_floor_usd` | `f64` | shared | reserved strategy setting |
| `strategy.gas_buffer_multiplier` | `f64` | shared | reserved strategy setting |
| `strategy.max_gas_price_gwei` | `f64` | shared | reserved strategy setting |
| `rpc.helius_api_key` | `string` | shared | reserved external RPC setting |
| `rpc.helius_rpc_url` | `string` | shared | reserved external RPC setting |
| `indexer.geyser_plugin_path` | `string` | indexer | expected path to the built plugin library |
| `indexer.database_url` | `string` | indexer | PostgreSQL connection string |
| `indexer.concurrency` | `usize` | indexer | configured worker concurrency value |
| `indexer.http_port` | `u16` | indexer | axum server listen port |
| `coordinator.redis_url` | `string` | coordinator and prover | Redis connection string |
| `coordinator.callback_timeout_seconds` | `u64` | shared config | callback timeout setting |
| `coordinator.max_concurrent_jobs` | `usize` | prover | semaphore limit for prover jobs |
| `coordinator.indexer_url` | `string` | coordinator | base URL for the indexer HTTP API |
| `prover.sp1_proving_key_path` | `string` | prover | configured proving-key path |
| `prover.groth16_params_path` | `string` | prover | configured Groth16 params path |
| `prover.mock_prover` | `bool` | prover | sets mock proving mode when `SP1_PROVER` is unset |
| `observability.log_level` | `string` | shared | tracing filter level |
| `observability.metrics_port` | `u16` | shared | metrics server port |

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
anchor test --skip-build
```

## limitations

the current repository has a few important gaps that matter when you evaluate readiness:

- the on-chain verifier registry only knows the built-in demo computation id
- the callback worker still sends empty `public_inputs`
- the historical-average template is implemented off-chain but not yet fully wired into on-chain verification
- `crates/sdk` is still a stub
- `tests/integration.rs` and `tests/property.rs` are placeholders for later phases

## contributing

see [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md).

## license

this project is licensed under the MIT license. see [LICENSE](LICENSE).