# Sonar SSOT

This document is the factual description of what exists in the repository today. It is intentionally narrower than the production target and should avoid aspirational language.

## Repo scope

Sonar currently includes all of the following in-tree:

| Area                     | Current state                                        |
| ------------------------ | ---------------------------------------------------- |
| On-chain program         | Anchor program in `program/`                         |
| CPI SDK                  | Implemented in `crates/sdk/`                         |
| Developer CLI            | Implemented in `crates/cli/`                         |
| Coordinator              | Implemented in `crates/coordinator/`                 |
| Prover                   | Implemented in `crates/prover/`                      |
| Indexer + Geyser plugin  | Implemented in `crates/indexer/`                     |
| Prod-oriented Compose    | Implemented in `docker-compose.prod.yml`             |
| Observability baseline   | Prometheus + Grafana config in-tree                  |
| Devnet deploy automation | Implemented in `scripts/deploy-devnet.sh`            |
| Example callback program | `echo_callback/`                                     |
| Example consumer         | `programs/historical_avg_client/`                    |
| Example SP1 guests       | `programs/fibonacci/` and `programs/historical_avg/` |

The workspace members are:

- `crates/common`
- `crates/cli`
- `crates/indexer`
- `crates/coordinator`
- `crates/prover`
- `crates/solana-program-shim`
- `crates/sdk`
- `program`
- `echo_callback`
- `programs/historical_avg_client`

## Toolchain and major dependencies

- Rust edition: 2021
- Anchor: `0.32.1`
- Solana crates: `2.3.x`
- SP1 SDK: `6.0.2`
- `groth16-solana`: `0.2.0`
- Axum: `0.7`
- SQLx: `0.8` with Postgres only
- Redis: `0.24`
- Node/TS test surface for Anchor integration tests

## What the on-chain program does today

The Sonar Anchor program supports four core instructions:

1. `register_verifier`
   - Creates a verifier registry PDA for a `computation_id`.
   - Stores Groth16 verifying-key material on-chain.
2. `request`
   - Creates `RequestMetadata` and `ResultAccount` PDAs.
   - Escrows the request fee in the request PDA.
   - Emits structured request and input logs for the coordinator.
3. `callback`
   - Verifies the submitted Groth16 proof against the registered verifier.
   - Writes the result to the result PDA.
   - Invokes the consumer callback program.
   - Pays the prover from the escrowed fee.
4. `refund`
   - Returns the escrowed fee to the payer after deadline expiry.

The program exports two computation IDs directly:

- `DEMO_COMPUTATION_ID`
- `HISTORICAL_AVG_COMPUTATION_ID`

Important nuance: dynamic verifier registration exists, but the repo only ships end-to-end proving logic for the computations implemented by the prover registry. Registering an arbitrary verifier is possible on-chain; making that computation runnable also requires matching off-chain prover support.

## Off-chain components that exist today

### Coordinator

The coordinator currently:

- subscribes to Sonar program logs over Solana WebSocket
- decodes request metadata and structured inputs
- enriches `historical_avg` jobs through the indexer HTTP API
- pushes prover work to Redis and later submits callback transactions

### Prover

The prover currently:

- resolves computations via an internal registry
- builds and runs SP1 guests
- wraps proofs into Groth16 when required
- supports artifact export for verifier registration
- supports a mock proving path for selected tests and CI flows

### Indexer

The indexer currently includes:

- a Geyser plugin that writes account updates into Postgres
- an Axum server exposing `GET /account_history/:pubkey?from_slot=<u64>&to_slot=<u64>`
- database helpers used by integration and end-to-end flows

## Operational assets that exist today

The repository includes the following operational helpers and topology files:

- `docker-compose.prod.yml` for `postgres`, `redis`, `coordinator`, `prover`, `prometheus`, and `grafana`
- `docker/prometheus/prometheus.yml` for scraping coordinator and prover metrics
- `scripts/deploy-devnet.sh` for repeatable Anchor workspace deployment to devnet
- `scripts/local-ci.sh` and `.actrc` for local GitHub Actions execution via `act`

Important current nuance: the prod-oriented Compose file does not run the indexer. `INDEXER_URL` defaults to `http://host.docker.internal:8080`, so the coordinator and prover currently expect an externally managed indexer API.

The coordinator and prover expose Prometheus metrics via:

- `COORDINATOR_METRICS_PORT` default `9090`
- `PROVER_METRICS_PORT` default `9091`

Grafana is exposed on host port `3000` by default through `GRAFANA_PORT`.

## Developer surfaces that exist today

### SDK

`crates/sdk` is not a stub. It provides:

- an Anchor CPI helper for Sonar `request`
- PDA derivation and validation for `request_metadata` and `result_account`
- exported program types and common computation IDs for downstream consumers

### CLI

`crates/cli` is not a placeholder. It currently provides:

- `sonar-cli register`
- ELF hashing to derive `computation_id`
- verifier artifact resolution and integrity checking
- construction and submission of `register_verifier`

### Exported artifacts

The repo includes a binary, `sonar-export-artifacts`, that writes verifier artifacts to disk by calling into the prover crate.

## Tested vertical slices

The strongest fully exercised vertical slice in the repo today is `historical_avg`:

- a consumer program submits a request
- the coordinator observes and enriches it
- the prover computes an average from indexed account history
- the callback verifies and stores the result
- the integration harness checks final on-chain state

There is also a simpler demo/fibonacci proving path used for proof generation and verifier/artifact flows.

## Quality, CI, and security automation

The repo currently has:

- Rust fmt + clippy enforcement
- workspace unit/integration tests
- Anchor build and Anchor test automation
- end-to-end historical-average CI coverage
- `cargo audit`
- `cargo deny`
- `gitleaks`
- Criterion benchmarks for coordinator and prover hot paths
- local `act` automation through `scripts/local-ci.sh`

For callback validation specifically, the current local default is now layered:

- cheap prover/unit coverage for Groth16 payload shape and malformed-input behavior
- deterministic fixture-backed `historical_avg` callback coverage for the ignored local stack e2e path when the prover is explicitly run in mock mode
- Anchor integration coverage for callback invariants
- heavyweight real SP1 Groth16 smoke kept as explicit opt-in coverage rather than the default local gate

## Current limitations

These are factual current gaps, not roadmap promises:

- the system is still best treated as devnet-grade rather than production-grade
- verifier lifecycle governance is manual
- operational runbooks, dashboards, tracing, alerts, and SLOs are not yet formalized in-repo
- external APIs and SDK ergonomics are still narrow and computation-specific
- the indexer API surface is intentionally small today
- the prod-oriented Compose topology still assumes an external indexer service
- full local CPU SP1 Groth16 proving remains too heavy for some developer machines, so the repo still depends on optional heavy-proof runners for the strongest live proof-generation coverage
