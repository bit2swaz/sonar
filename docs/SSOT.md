# ssot

## purpose

this file is the single source of truth for what the `sonar` repository actually implements today.

it is intentionally narrower than a vision document. every statement below is based on checked-in code, config, tests, or ci workflows in this repository.

## how this fits with the newer planning docs

this file is the factual "what exists now" document.

for the forward-looking production direction and execution plan, use:

- [PROD_TARGET.md](PROD_TARGET.md) for the architecture Sonar is moving toward
- [ROADMAP.md](ROADMAP.md) for the canonical transition plan from the current MVP to that target

## repository scope

the repository contains:

- one on-chain Anchor program in `program/`
- one test-only callback program in `echo_callback/`
- three runtime binaries in `bin/`
  - `sonar-indexer`
  - `sonar-coordinator`
  - `sonar-prover`
- shared crates in `crates/common`, `crates/sdk`, and `crates/solana-program-shim`
- SP1 guest programs in `programs/`

the workspace members declared in `Cargo.toml` are:

- `crates/common`
- `crates/indexer`
- `crates/coordinator`
- `crates/prover`
- `crates/solana-program-shim`
- `crates/sdk`
- `program`
- `echo_callback`

## pinned toolchain and major dependencies

current pinned or explicitly selected versions:

- Rust toolchain: `1.94.1`
- Anchor CLI and crate family: `0.32.1`
- Solana CLI in ci: `3.0.13`
- `solana-sdk`: `2.3.1`
- `solana-program`: `2.3.0`
- `sp1-sdk`: `6.0.2`
- `groth16-solana`: `0.2.0`
- `sqlx`: `0.8` with PostgreSQL only
- `redis`: `0.24`
- `axum`: `0.7`
- `reqwest`: `0.12`

## deployed and declared program ids

from `Anchor.toml` and the on-chain code:

### localnet

- `sonar_program = EE2sQ2VRa1hY3qjPQ1PEwuPZX6dGwTZwHMCumWrGn3sV`
- `echo_callback = 3RBU9G6Mws9nS8bQPg2cVRbS2v7CgsjAvv2MwmTcmbyA`

### devnet

- `sonar = 5B1rXQ71oEWUPc3AemCBTQtb5pmGAnX1jbGvZKcgBy84`

### program source ids

- `program/src/lib.rs` declares `EE2sQ2VRa1hY3qjPQ1PEwuPZX6dGwTZwHMCumWrGn3sV`
- `echo_callback/src/lib.rs` declares `3RBU9G6Mws9nS8bQPg2cVRbS2v7CgsjAvv2MwmTcmbyA`

the source id for `sonar` matches the `sonar_program` localnet entry in `Anchor.toml`.

## architecture

the current runtime model is:

1. a client calls the on-chain `request` instruction
2. the program stores request metadata and emits structured logs
3. the coordinator listens for those logs over Solana websocket subscriptions
4. the coordinator fetches request metadata and pushes a `ProverJob` into Redis
5. the prover consumes the job, resolves the computation id, runs the SP1 guest, and pushes a `ProverResponse`
6. the coordinator callback worker consumes the response and submits the on-chain `callback` instruction

for the historical-average template there is one extra step:

- the coordinator calls the indexer HTTP endpoint to turn the raw `(pubkey, from_slot, to_slot)` request bytes into a bincode-encoded `Vec<u64>` before dispatching the prover job

## on-chain program

the `sonar` program implements three instructions:

- `request`
- `callback`
- `refund`

### `request`

`request` does the following:

- checks that the deadline is in the future
- checks that the fee is non-zero
- initializes `request_metadata` and `result_account` PDAs
- transfers lamports into the request metadata account
- emits:
  - `sonar:request:<hex request id>`
  - `sonar:inputs:<hex encoded raw inputs>`

### `callback`

`callback` does the following:

- requires the request to be pending
- requires the current slot to be before or at the deadline
- verifies a Groth16 proof
- writes the result bytes into `result_account`
- marks the request as completed
- cpies into the configured callback program using a fixed `sonar_callback` discriminator
- transfers the escrowed fee to the prover signer

### `refund`

`refund` does the following:

- requires the original payer signer
- requires the request to still be pending
- requires the deadline to have passed
- returns the escrowed fee to the payer
- marks the request as refunded

### on-chain data layout

`RequestMetadata` stores:

- `request_id`
- `payer`
- `callback_program`
- `result_account`
- `computation_id`
- `deadline`
- `fee`
- `status`
- `completed_at`
- `bump`

`ResultAccount` stores:

- `request_id`
- `result`
- `is_set`
- `written_at`
- `bump`

## current verifier registry

the on-chain verifier logic currently recognizes two computation ids:

- `DEMO_COMPUTATION_ID`
- `HISTORICAL_AVG_COMPUTATION_ID`

the `historical_avg` path is still an MVP-specific verifier path rather than a finished production verifier rollout. the code currently routes `HISTORICAL_AVG_COMPUTATION_ID` into `verify_historical_avg_proof_mvp` and aliases `HISTORICAL_AVG_VERIFYING_KEY` to the demo verifying key.

## off-chain services

### indexer

the indexer crate provides:

- a `cdylib` Geyser plugin export through `_create_plugin`
- embedded SQLx migrations
- PostgreSQL account-history storage
- an axum HTTP server

the query endpoint is:

- `GET /account_history/:pubkey?from_slot=<u64>&to_slot=<u64>`

responses are JSON arrays of lamport balances in ascending `(slot, write_version)` order.

### coordinator

the coordinator binary starts two long-running tasks:

- a listener task
- a callback worker task

the listener:

- subscribes to Solana logs mentioning the sonar program id
- parses `sonar:request:` and `sonar:inputs:` log lines
- fetches the request metadata account over RPC
- builds a `ProverJob`
- pushes that job to Redis list `sonar:jobs`

the callback worker:

- pops `ProverResponse` values from Redis list `sonar:responses`
- fetches request metadata to recover the callback program pubkey
- builds the `callback` instruction payload manually
- sends the transaction with retries

important current limitation:

- the callback worker now forwards `response.public_inputs`, but the historical-average on-chain path is still an MVP verifier flow rather than a final production verifier implementation

### prover

the prover service:

- pops JSON jobs from Redis list `sonar:jobs`
- resolves the computation id to an ELF path
- executes the guest with SP1
- wraps the proof output
- pushes JSON responses into Redis list `sonar:responses`

the prover registry currently contains two computations:

- `fibonacci`
- `historical_avg`

computation ids are derived as `sha256(elf bytes)`.

## historical-average template

the repository contains a historical-average proving path with these parts:

- guest crate in `programs/historical_avg/`
- committed ELF in `programs/historical_avg/elf/historical-avg-program`
- prover registry entry in `crates/prover/src/registry.rs`
- SP1 execution path in `crates/prover/src/sp1_wrapper.rs`
- coordinator input enrichment in `crates/coordinator/src/listener.rs`
- indexer HTTP route in `crates/indexer/src/server.rs`

the guest itself is simple:

- input: `Vec<u64>`
- output: integer average as `u64`

the current historical-average flow is only partially complete because:

- the on-chain historical-average path still uses an MVP verification helper rather than a final dedicated production verifier rollout
- the repository's strongest end-to-end historical-average proof remains local/development oriented rather than a hardened production deployment path
- `config/devnet.toml` is still out of date for the current config shape

## configuration model

the shared config type lives in `crates/common/src/config.rs`.

required sections are:

- `network`
- `strategy`
- `rpc`
- `indexer`
- `coordinator`
- `prover`
- `observability`

important fields added by the current code shape:

- `indexer.http_port`
- `coordinator.indexer_url`

the checked-in `config/default.toml` matches the current config struct.

the checked-in `config/devnet.toml` does not yet match the current config struct because it is missing:

- `indexer.http_port`
- `coordinator.indexer_url`

binary-specific env vars:

- `SONAR_CONFIG` is used by `sonar-indexer` and `sonar-prover`
- `SONAR_CONFIG_PATH` is used by `sonar-coordinator`
- `SONAR_COORDINATOR_KEYPAIR_PATH` optionally points to a Solana keypair JSON file
- `SP1_PROVER` overrides prover mode

## queue and wire formats

shared queue types in `crates/common/src/types.rs`:

- `ProverJob`
- `ProverResponse`

Redis list names in `crates/coordinator/src/dispatcher.rs` and `crates/prover/src/service.rs`:

- `sonar:jobs`
- `sonar:responses`

historical-average raw request input format in the coordinator listener:

- `pubkey[32] + from_slot[8] + to_slot[8]`

historical-average prover input format after enrichment:

- `bincode::serialize(&Vec<u64>)`

## test coverage that exists today

the repository contains:

- Rust unit tests across `crates/common`, `crates/coordinator`, `crates/indexer`, `crates/prover`, and `program`
- a TypeScript integration suite in `program/tests/sonar.ts`
- a checked-in Rust historical-average e2e test in `tests/e2e_historical_avg.rs`
- PostgreSQL-backed indexer tests that use Docker
- prover service tests for queue behavior and concurrency

the repository also contains placeholders for later phases:

- `tests/integration.rs`
- `tests/property.rs`

## ci behavior

the ci workflow currently runs:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace -- --skip integration`
- `cargo test --test e2e_historical_avg -- --ignored --nocapture`
- `cargo audit`
- `cargo deny check`
- `anchor build`
- `anchor test` with output checks that ignore Anchor CLI's known trailing cleanup `os error 2`
- `./scripts/verify-demo.sh`

## current implementation status by phase

based on the checked-in code and tests:

- phase 0 - complete
- phase 1 - complete
- phase 2 - complete for the demo verifier path used in Anchor tests
- phase 3 - complete for the current prover scaffolding and tests
- phase 4 - complete for the current indexer and PostgreSQL path
- phase 5 - complete for the current coordinator and Redis queue path
- phase 6 - partially complete
- phase 7 and later - not implemented yet

## non-goals for the current repository state

the following ideas may appear in older planning material but are not implemented in code here today:

- staking or slashing
- a permissionless prover set
- a token
- recursive aggregation across many requests
- a production-grade rust sdk
- a production-grade ts sdk or cli
- cross-chain proofs

## summary

sonar currently provides a solid local and ci-tested baseline for:

- on-chain request lifecycle management
- Redis-backed job dispatch and response handling
- PostgreSQL-backed historical state lookup
- SP1-backed computation execution for registered ELFs

the strongest on-chain verifier coverage in the codebase is still the built-in demo Groth16 path exercised by the Anchor integration tests. in addition, the repository now contains a working local historical-average MVP flow with checked-in e2e and demo verification, but that path is still not a finished production-grade verifier rollout.