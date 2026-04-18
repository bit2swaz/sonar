# Sonar

Sonar is a Solana-native ZK coprocessor prototype built around an Anchor program, an off-chain proving pipeline, and a thin developer surface for submitting requests and registering verifiers. The repo now contains a full request -> prove -> callback -> index loop, a real CPI SDK, a developer CLI for verifier registration, Criterion benchmarks for hot paths, a repeatable devnet deployment script, and a baseline observability stack for the production-oriented Compose topology.

Sonar is not production-ready yet. The current state is best described as a hardened devnet-quality system with one end-to-end vertical slice (`historical_avg`) and the core primitives needed to expand toward a multi-computation coprocessor.

## Current status

- On-chain program supports `register_verifier`, `request`, `callback`, and `refund`.
- Off-chain services include a coordinator, prover, indexer, and Geyser plugin.
- `historical_avg` runs end-to-end with an ignored integration test and CI coverage.
- `crates/sdk` provides a real Anchor CPI helper for downstream programs.
- `crates/cli` provides `sonar-cli register` for verifier registration.
- `scripts/deploy-devnet.sh` automates wallet checks, build, deploy, and post-deploy verification on devnet.
- `docker-compose.prod.yml` now includes Prometheus and Grafana alongside the coordinator/prover stack.
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
    D -. metrics .-> M[Prometheus]
    G -. metrics .-> M
    M --> N[Grafana]
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
| `docker/`                         | Container build files plus Prometheus scrape config for the prod-oriented stack        |
| `scripts/`                        | Devnet deployment, local CI, and demo verification helpers                             |
| `.github/workflows/`              | CI, e2e, demo verification, audit, deny, and secret-scanning automation                |
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
bash scripts/anchor-test.sh
```

For the full historical-average vertical slice:

```bash
cargo build --bins
cargo build -p sonar-indexer --lib
anchor build
cargo test --test e2e_historical_avg -- --ignored --nocapture
```

For expensive real-proof smoke coverage that is not part of the default local gate:

```bash
cargo test -p sonar-prover --lib \
  tests::test_extract_sp1_groth16_payload_matches_real_sp1_shape_live_smoke \
  -- --ignored --exact --nocapture
```

That prover smoke is intentionally opt-in. On lower-memory no-swap laptops it may be SIGKILLed even when the callback path is logically correct.

For the prod-oriented off-chain stack with baseline observability:

```bash
docker compose -f docker-compose.prod.yml up --build
```

That stack launches `postgres`, `redis`, `coordinator`, `prover`, `prometheus`, and `grafana`. It expects an indexer API to be reachable at `INDEXER_URL` and exposes Grafana on `http://localhost:3000` by default.

## Common workflows

### Rust quality gates

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --skip integration
```

### Local GitHub Actions CI

Install `act` to run the GitHub Actions workflows locally before pushing:

```bash
brew install act
```

Or via the upstream install script:

```bash
curl https://raw.githubusercontent.com/nektos/act/master/install.sh | sudo bash
```

The repo includes `.actrc` with a `ubuntu-latest` mapping to the fuller
`catthehacker/ubuntu:full-latest` runner image so local `act` runs match the
GitHub-hosted environment more closely and avoid re-downloading core toolchains.

Before your first local run:

```bash
cp .secrets.example .secrets
```

If you want to run workflows that require GitHub-authenticated actions or API
access, replace the dummy `GITHUB_TOKEN` in `.secrets` with a real token first.

Then execute the local CI wrapper:

```bash
scripts/local-ci.sh
```

The script checks that Docker is installed and running, then invokes `act`
with `.secrets`, cached action content, and cached runner images when possible.
You can pass any normal `act` arguments through the wrapper, for example:

```bash
scripts/local-ci.sh -l
scripts/local-ci.sh pull_request
scripts/local-ci.sh -W .github/workflows/ci.yml -j check
```

If your environment requires authentication to pull the configured runner image, run `docker login` before starting local `act` runs.

### Anchor validation

Use the repo wrapper instead of invoking `anchor test` directly:

```bash
bash scripts/anchor-test.sh
```

That wrapper runs `anchor build`, syncs the `target/idl/sonar_program.json`
alias that Anchor's log-stream path expects, and then executes
`anchor test --skip-build`.

### Devnet deployment

Use the deployment helper when you want a repeatable devnet release of the Anchor workspace:

```bash
scripts/deploy-devnet.sh
```

The script:

- resolves the deploy wallet from `ANCHOR_WALLET` or `Anchor.toml`
- ensures the wallet has at least 2 SOL on devnet, requesting an airdrop if needed
- runs `anchor keys sync`
- builds the workspace with a platform-tools fallback when needed
- deploys to devnet and verifies the deployed program account

### Observability stack

- `docker/prometheus/prometheus.yml` scrapes the coordinator on `9090` and prover on `9091`
- `docker-compose.prod.yml` wires Grafana to host port `3000`
- metrics ports stay configurable through `COORDINATOR_METRICS_PORT` and `PROVER_METRICS_PORT`

### Benchmarks

```bash
cargo bench -p sonar-coordinator
cargo bench -p sonar-prover
```

### Proof-validation workflow

- Default local validation should rely on the fast unit and Anchor integration suites.
- Expensive real SP1 Groth16 smoke tests are opt-in and are better suited to higher-memory or GPU-capable machines, or CI/nightly coverage.
- Run `bash scripts/anchor-test.sh` for the Anchor suite; it handles the repo's `sonar` versus `sonar_program` IDL-name mismatch before invoking Anchor.

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
- `docker-compose.prod.yml` defaults `INDEXER_URL` to `http://host.docker.internal:8080`, so the current prod-oriented Compose stack assumes the indexer is managed outside that file.
- Grafana credentials and host port are controlled through `GRAFANA_ADMIN_USER`, `GRAFANA_ADMIN_PASSWORD`, and `GRAFANA_PORT`.

## Validation and security automation

- CI: format, clippy, unit/integration tests, Anchor build/tests, e2e flow, and demo verification.
- Security workflow: `cargo audit`, `cargo deny`, and `gitleaks`.
- Pre-commit hooks: Rust fmt/clippy, `cargo deny`, `cargo audit`, and Prettier for Markdown/JSON/YAML.
- Local CI runs go through `scripts/local-ci.sh`, `.secrets`, and the `.actrc` mapping for `ubuntu-latest`.

## Limits of the current repo

- Production economics, fee policy, and capacity planning are still evolving.
- Verifier registration exists, but operational lifecycle management and governance are still manual.
- The repo proves one strong vertical slice today rather than a broad catalog of production computations.
- The system is still oriented around devnet/local-validator workflows, not a hardened mainnet rollout.
- Observability is still baseline-only: metrics scraping and Grafana exist, but dashboards, alerts, tracing, and runbooks remain incomplete.

## Read next

- `docs/SSOT.md` for the current implementation truth
- `docs/ARCHITECTURE.md` for component and lifecycle detail
- `docs/ROADMAP.md` for what is done versus what remains
- `docs/PROD_TARGET.md` for the desired production shape
- `docs/CONTRIBUTING.md` for local workflow expectations
- `SECURITY.md` for disclosure and secure-development guidance
