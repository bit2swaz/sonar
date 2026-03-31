# contributing

thanks for contributing to sonar.

this repository mixes Rust, Anchor, Solana tooling, TypeScript tests, PostgreSQL, Redis, and SP1-based proving code. the quickest way to stay productive is to match the pinned tool versions in the repo and run the same checks as ci.

## prerequisites

install these tools before you start:

| tool | required version or note | why |
| --- | --- | --- |
| Rust | `1.94.1` from `rust-toolchain.toml` | workspace builds, tests, clippy, rustfmt |
| rust components | `rustfmt`, `clippy`, `rust-src` | formatting, linting, sbf builds |
| Solana CLI | `3.0.13` | local validator and Anchor workflow |
| Anchor CLI | `0.32.1` | build and test the on-chain programs |
| Node.js | `20.x` is used in ci | TypeScript Anchor tests |
| npm | any recent version that works with Node 20 | installs test dependencies |
| PostgreSQL | local instance or Docker container | indexer storage tests and runtime |
| Redis | local instance | coordinator and prover queues |
| Docker | optional but recommended | PostgreSQL-backed indexer tests |
| protobuf-compiler | required in ci | transitive Rust build dependency |
| pkg-config and libudev-dev | required for Anchor CLI install in ci | native build prerequisites |

optional but useful:

- `pre-commit`
- `cargo-audit`
- `cargo-deny`

## clone and bootstrap

```bash
git clone git@github.com:bit2swaz/sonar.git
cd sonar
rustup toolchain install 1.94.1 --component rustfmt clippy rust-src
npm install
```

install Solana and Anchor with the versions used by the repo:

```bash
sh -c "$(curl -sSfL https://release.anza.xyz/v3.0.13/install)"
cargo install anchor-cli --version 0.32.1 --locked
```

if you want git hooks:

```bash
./scripts/install-hooks.sh
pre-commit install
```

## local services

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

export the minimum environment expected by `config/default.toml`:

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

notes:

- the prover can run in mock mode through `config/default.toml` or by setting `SP1_PROVER=mock`
- `bin/coordinator.rs` reads `SONAR_CONFIG_PATH`
- `bin/indexer.rs` and `bin/prover.rs` read `SONAR_CONFIG`
- `config/devnet.toml` is currently missing the newer phase 6 keys, so `config/default.toml` is the safest starting point

## build commands

build the Rust workspace:

```bash
cargo build --workspace
```

build the Solana program with Anchor:

```bash
anchor build
```

or use the sbf helper script that forces the newer Solana platform tools version:

```bash
./scripts/build-program.sh
```

## code style

use the same checks as ci before pushing:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --skip integration
cargo audit
cargo deny check
```

formatting and linting rules in the repo:

- Rust formatting uses `.rustfmt.toml`
- clippy warnings are treated as errors in ci
- prettier runs on markdown, json, and yaml through `.pre-commit-config.yaml`
- pre-commit hooks also run `cargo deny` and `cargo audit`

## running tests

### Rust unit and service tests

```bash
cargo test --workspace -- --skip integration
```

this covers:

- shared config and type tests
- coordinator parsing and queue tests
- indexer database and http tests
- prover registry, wrapper, and service tests
- program unit tests compiled as part of the workspace

### Anchor integration tests

start a local validator and run the TypeScript suite:

```bash
solana-test-validator --quiet &
anchor build
anchor test --skip-build
```

the TypeScript suite in `program/tests/sonar.ts` exercises request, callback, refund, access control, and edge cases using the `echo_callback` helper program.

### ignored integration placeholder

ci also runs an ignored Rust integration target:

```bash
cargo test --workspace --test integration -- --ignored
```

at the moment `tests/integration.rs` is only a placeholder for later phases.

### PostgreSQL-backed indexer tests

some indexer tests start a temporary Dockerized PostgreSQL instance. make sure Docker is available before running the full Rust test suite.

## running the services

run the indexer http server:

```bash
SONAR_CONFIG=config/default.toml cargo run --bin sonar-indexer
```

run the prover service:

```bash
SONAR_CONFIG=config/default.toml cargo run --bin sonar-prover
```

run the coordinator:

```bash
SONAR_CONFIG_PATH=config/default.toml cargo run --bin sonar-coordinator
```

if you need a persistent signer for callback transactions, set:

```bash
export SONAR_COORDINATOR_KEYPAIR_PATH=$HOME/.config/solana/id.json
```

## pull request process

before opening a pull request:

1. rebase on the latest `main`
2. run the full check set locally
3. update docs when behavior, config, or workflows change
4. keep changes focused and explain any tradeoffs or follow-up work

for the pull request description, include:

- what changed
- why it changed
- how you tested it
- any known limitations or follow-up items

## what to document in code review

call out these cases explicitly:

- changes to account layouts or instruction data
- changes to queue payloads or Redis key names
- new config fields or renamed env vars
- any change that affects verifier assumptions or callback semantics
- any ci or developer workflow changes

## security notes

keep secrets out of the repository:

- never commit real keypairs or API keys
- use `.env.example` as the shape reference only
- use the checked-in secret scanning and dependency scanning workflows
- do not deploy `echo_callback` to mainnet

## where to start

if you are new to the codebase, read these files in order:

1. `README.md`
2. `docs/ARCHITECTURE.md`
3. `docs/SSOT.md`
4. `program/src/lib.rs`
5. `crates/coordinator/src/listener.rs`
6. `crates/prover/src/service.rs`
7. `crates/indexer/src/db.rs`