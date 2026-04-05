# Sonar Architecture

This document describes the current architecture implemented in the repository.

## High-level component map

```mermaid
flowchart LR
    subgraph Solana
        P[Sonar program]
        S[(RequestMetadata PDA)]
        R[(ResultAccount PDA)]
        V[(VerifierRegistry PDA)]
        Cb[Consumer callback program]
    end

    subgraph Offchain
        L[Coordinator listener]
        Q[(Redis queues)]
        W[Coordinator callback worker]
        Pr[Prover]
        Gp[Geyser plugin]
        Db[(Postgres)]
        Api[Indexer HTTP API]
    end

    U[Client or CPI caller] -->|request| P
    P --> S
    P --> R
    P -->|logs| L
    L -->|historical_avg enrichment| Api
    Api --> Db
    Gp --> Db
    L -->|ProverJob| Q
    Q --> Pr
    Pr -->|ProverResponse| Q
    Q --> W
    W -->|callback + verifier account| P
    P -->|verify proof| V
    P -->|invoke callback| Cb
```

## Request lifecycle

```mermaid
sequenceDiagram
    participant User as Client or CPI caller
    participant Sonar as Sonar program
    participant Listener as Coordinator listener
    participant API as Indexer HTTP API
    participant Redis as Redis
    participant Prover as Prover
    participant Worker as Callback worker
    participant Consumer as Callback program

    User->>Sonar: request(request_id, computation_id, inputs, deadline, fee)
    Sonar->>Sonar: create RequestMetadata + ResultAccount
    Sonar-->>Listener: emit request and inputs logs
    Listener->>API: fetch account history when computation needs enrichment
    Listener->>Redis: enqueue ProverJob
    Prover->>Redis: consume ProverJob
    Prover->>Prover: run SP1 guest and wrap proof
    Prover->>Redis: publish ProverResponse
    Worker->>Redis: consume ProverResponse
    Worker->>Sonar: callback(proof, public_inputs, result)
    Sonar->>Sonar: verify Groth16 proof against VerifierRegistry
    Sonar->>Sonar: write result and mark request completed
    Sonar->>Consumer: invoke consumer callback
    Sonar->>Worker: transfer prover fee
```

## Main components

| Component                         | Responsibility                                                            |
| --------------------------------- | ------------------------------------------------------------------------- |
| `program/`                        | Owns request/result/verifier state and enforces proof verification        |
| `crates/sdk/`                     | Makes request CPI calls ergonomic for downstream Anchor programs          |
| `crates/cli/`                     | Registers verifier keys derived from exported artifacts and ELF hashes    |
| `crates/coordinator/`             | Watches logs, enriches jobs, enqueues proving work, and submits callbacks |
| `crates/prover/`                  | Resolves computations, runs SP1, wraps proofs, exports artifacts          |
| `crates/indexer/`                 | Persists chain data and exposes the account-history query surface         |
| `programs/historical_avg_client/` | Example consumer of the Sonar request path                                |
| `echo_callback/`                  | Minimal callback program used for testing and orchestration               |

## On-chain data model

### `RequestMetadata`

Tracks:

- `request_id`
- payer
- callback program
- result account
- computation ID
- deadline
- fee
- status (`Pending`, `Completed`, `Refunded`)
- completion slot

### `ResultAccount`

Tracks:

- `request_id`
- raw result bytes
- whether the result has been set
- write slot

### `VerifierRegistry`

Tracks:

- `computation_id`
- authority
- Groth16 verifying key material
- PDA bump

## Computation model

Two computation IDs are exported directly by the program today:

- `DEMO_COMPUTATION_ID`
- `HISTORICAL_AVG_COMPUTATION_ID`

The crucial architectural distinction is:

- on-chain verification is generic over registered verifier material
- off-chain proving is only available for computations implemented in the prover registry

That means verifier registration is dynamic, but end-to-end support still depends on off-chain computation implementations and artifacts being present.

## Historical-average specialization

`historical_avg` is the most complete end-to-end use case in the repo.

Its special path looks like this:

1. the client submits a request whose raw inputs encode `(pubkey, from_slot, to_slot)`
2. the coordinator parses those inputs from program logs
3. the coordinator fetches balance history from the indexer API
4. the fetched balances become the serialized prover inputs
5. the prover computes the average and produces the proof/result bundle
6. the callback writes the final value on-chain

This split demonstrates how Sonar can combine on-chain triggers with off-chain data enrichment while preserving an on-chain verification step.

## Trust boundaries

### Trusted for correctness

- the Sonar program's account constraints and proof verification
- the registered verifier material for a computation ID
- Solana state transitions once transactions finalize

### Trusted for liveness, not correctness

- coordinator availability
- Redis availability
- prover availability
- indexer freshness and HTTP availability

If off-chain services fail, the system should degrade toward timeout/refund rather than silent correctness failure.

## Failure model today

Current expected failure outcomes:

- missed or stalled off-chain processing -> request remains pending until refund path becomes valid
- malformed or mismatched verifier/public inputs -> callback fails on-chain
- stale or unavailable enrichment data -> coordinator/prover job fails, no incorrect result is committed
- worker or queue restart -> jobs may require replay/retry procedures outside the current minimal automation

## Operational notes

- The indexer API is intentionally narrow: account-history lookups only.
- The current queueing model is Redis-based and simple by design.
- CI validates both Rust-native flows and Anchor-based flows.
- Benchmarks currently focus on coordinator and prover hot paths, not full-system load.# Sonar Architecture

This document describes the current architecture implemented in the repository.

## High-level component map

```mermaid
flowchart LR
    subgraph Solana
        P[Sonar program]
        S[(RequestMetadata PDA)]
        R[(ResultAccount PDA)]
        V[(VerifierRegistry PDA)]
        Cb[Consumer callback program]
    end

    subgraph Offchain
        L[Coordinator listener]
        Q[(Redis queues)]
        W[Coordinator callback worker]
        Pr[Prover]
        Gp[Geyser plugin]
        Db[(Postgres)]
        Api[Indexer HTTP API]
    end

    U[Client or CPI caller] -->|request| P
    P --> S
    P --> R
    P -->|logs| L
    L -->|historical_avg enrichment| Api
    Api --> Db
    Gp --> Db
    L -->|ProverJob| Q
    Q --> Pr
    Pr -->|ProverResponse| Q
    Q --> W
    W -->|callback + verifier account| P
    P -->|verify proof| V
    P -->|invoke callback| Cb
```

## Request lifecycle

```mermaid
sequenceDiagram
    participant User as Client or CPI caller
    participant Sonar as Sonar program
    participant Listener as Coordinator listener
    participant API as Indexer HTTP API
    participant Redis as Redis
    participant Prover as Prover
    participant Worker as Callback worker
    participant Consumer as Callback program

    User->>Sonar: request(request_id, computation_id, inputs, deadline, fee)
    Sonar->>Sonar: create RequestMetadata + ResultAccount
    Sonar-->>Listener: emit request and inputs logs
    Listener->>API: fetch account history when computation needs enrichment
    Listener->>Redis: enqueue ProverJob
    Prover->>Redis: consume ProverJob
    Prover->>Prover: run SP1 guest and wrap proof
    Prover->>Redis: publish ProverResponse
    Worker->>Redis: consume ProverResponse
    Worker->>Sonar: callback(proof, public_inputs, result)
    Sonar->>Sonar: verify Groth16 proof against VerifierRegistry
    Sonar->>Sonar: write result and mark request completed
    Sonar->>Consumer: invoke consumer callback
    Sonar->>Worker: transfer prover fee
```

## Main components

| Component                         | Responsibility                                                            |
| --------------------------------- | ------------------------------------------------------------------------- |
| `program/`                        | Owns request/result/verifier state and enforces proof verification        |
| `crates/sdk/`                     | Makes request CPI calls ergonomic for downstream Anchor programs          |
| `crates/cli/`                     | Registers verifier keys derived from exported artifacts and ELF hashes    |
| `crates/coordinator/`             | Watches logs, enriches jobs, enqueues proving work, and submits callbacks |
| `crates/prover/`                  | Resolves computations, runs SP1, wraps proofs, exports artifacts          |
| `crates/indexer/`                 | Persists chain data and exposes the account-history query surface         |
| `programs/historical_avg_client/` | Example consumer of the Sonar request path                                |
| `echo_callback/`                  | Minimal callback program used for testing and orchestration               |

## On-chain data model

### `RequestMetadata`

Tracks:

- `request_id`
- payer
- callback program
- result account
- computation ID
- deadline
- fee
- status (`Pending`, `Completed`, `Refunded`)
- completion slot

### `ResultAccount`

Tracks:

- `request_id`
- raw result bytes
- whether the result has been set
- write slot

### `VerifierRegistry`

Tracks:

- `computation_id`
- authority
- Groth16 verifying key material
- PDA bump

## Computation model

Two computation IDs are exported directly by the program today:

- `DEMO_COMPUTATION_ID`
- `HISTORICAL_AVG_COMPUTATION_ID`

The crucial architectural distinction is:

- on-chain verification is generic over registered verifier material
- off-chain proving is only available for computations implemented in the prover registry

That means verifier registration is dynamic, but end-to-end support still depends on off-chain computation implementations and artifacts being present.

## Historical-average specialization

`historical_avg` is the most complete end-to-end use case in the repo.

Its special path looks like this:

1. the client submits a request whose raw inputs encode `(pubkey, from_slot, to_slot)`
2. the coordinator parses those inputs from program logs
3. the coordinator fetches balance history from the indexer API
4. the fetched balances become the serialized prover inputs
5. the prover computes the average and produces the proof/result bundle
6. the callback writes the final value on-chain

This split demonstrates how Sonar can combine on-chain triggers with off-chain data enrichment while preserving an on-chain verification step.

## Trust boundaries

### Trusted for correctness

- the Sonar program's account constraints and proof verification
- the registered verifier material for a computation ID
- Solana state transitions once transactions finalize

### Trusted for liveness, not correctness

- coordinator availability
- Redis availability
- prover availability
- indexer freshness and HTTP availability

If off-chain services fail, the system should degrade toward timeout/refund rather than silent correctness failure.

## Failure model today

Current expected failure outcomes:

- missed or stalled off-chain processing -> request remains pending until refund path becomes valid
- malformed or mismatched verifier/public inputs -> callback fails on-chain
- stale or unavailable enrichment data -> coordinator/prover job fails, no incorrect result is committed
- worker or queue restart -> jobs may require replay/retry procedures outside the current minimal automation

## Operational notes

- The indexer API is intentionally narrow: account-history lookups only.
- The current queueing model is Redis-based and simple by design.
- CI validates both Rust-native flows and Anchor-based flows.
- Benchmarks currently focus on coordinator and prover hot paths, not full-system load.# architecture

## overview

sonar is a Solana zk coprocessor split across four main runtime components:

- an on-chain Anchor program in `program/`
- an indexer crate in `crates/indexer/`
- a coordinator crate in `crates/coordinator/`
- a prover crate in `crates/prover/`

the current repository already implements the request, queueing, proof generation, and callback submission building blocks. the historical-average template is partially wired end to end, but the on-chain verifier registry still only accepts the built-in demo computation id.

## component map

```mermaid
flowchart LR
    user[user program or test client]
    sonar[sonar anchor program]
    logs[solana logs and accounts]
    coord[coordinator]
    redis[(redis)]
    indexer[indexer]
    pg[(postgresql)]
    prover[prover]
    callback[callback transaction]

    user -->|request instruction| sonar
    sonar -->|request metadata + result pda| logs
    sonar -->|sonar:request and sonar:inputs logs| coord
    coord -->|fetch request metadata| logs
    coord -->|for historical_avg: http query| indexer
    indexer -->|read balances| pg
    coord -->|push proverjob json| redis
    prover -->|blpop sonar:jobs| redis
    prover -->|rpush proverresponse json| redis
    coord -->|blpop sonar:responses| redis
    coord -->|submit callback tx| callback
    callback --> sonar
```

## on-chain program

### purpose

the Anchor program in `program/src/lib.rs` is the trust anchor for result delivery. it stores request metadata, escrows the attached fee, verifies a Groth16 proof, writes the result account, invokes a callback program, and releases the fee to the prover.

### instructions

#### `request`

`request`:

- checks that `deadline > current_slot`
- checks that `fee > 0`
- creates `request_metadata` and `result_account` PDAs
- transfers lamports from the payer into `request_metadata`
- emits two logs:
  - `sonar:request:<hex request id>`
  - `sonar:inputs:<hex encoded raw inputs>`

#### `callback`

`callback`:

- requires the request status to be `pending`
- requires the deadline not to have passed
- verifies the Groth16 proof against the registered verifier key
- writes the result bytes to `result_account`
- marks the request as `completed`
- cpies into the configured callback program using the `sonar_callback` discriminator
- transfers the escrowed fee to the prover signer

#### `refund`

`refund`:

- requires the original payer signer
- requires the request to still be `pending`
- requires the current slot to be greater than `deadline`
- returns the escrowed fee to the payer
- marks the request as `refunded`

### account model

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

### verifier registry

the verifier registry in `program/src/verifier_registry.rs` currently exposes a single built-in demo verifier:

- `DEMO_COMPUTATION_ID`
- `DEMO_VERIFYING_KEY`
- `DEMO_PUBLIC_INPUTS_LEN = 9`

this means the on-chain program does not yet accept the historical-average computation id produced by the prover registry.

## indexer

### purpose

the indexer crate serves two roles:

- a loadable Geyser plugin through `cdylib`
- a normal Rust library and binary for database access and an HTTP query API

### storage

`crates/indexer/src/db.rs` manages PostgreSQL access. the schema is created from the embedded migration `202603310001_init_account_history.sql`.

main operations:

- connect to PostgreSQL with SQLx
- run embedded migrations
- insert batched account updates into `account_history`
- upsert slot metadata into `slot_metadata`
- query ordered account history
- query the latest snapshot at a slot
- query lamport balances for a pubkey and slot range

### http api

`crates/indexer/src/server.rs` exposes:

- `GET /account_history/:pubkey?from_slot=<u64>&to_slot=<u64>`

behavior:

- `:pubkey` is base58-decoded
- the handler rejects invalid base58 and wrong-length keys with `400`
- successful responses return `json` containing `Vec<u64>` lamport balances in slot and write-version order

### binary

`bin/indexer.rs`:

- loads config through `sonar_common::config::Config`
- connects to PostgreSQL
- runs migrations
- starts the axum server on `indexer.http_port`

## coordinator

### purpose

the coordinator is the bridge between Solana, the indexer, Redis, and the callback transaction path.

### listener path

`crates/coordinator/src/listener.rs`:

- subscribes to Solana websocket logs mentioning the sonar program id
- parses `sonar:request:` logs into request ids
- parses `sonar:inputs:` logs into raw inputs
- fetches the `RequestMetadata` account over RPC
- builds a `ProverJob`
- pushes JSON jobs to `sonar:jobs`

for historical-average requests, the listener also:

- decodes the 48-byte raw input payload as `pubkey + from_slot + to_slot`
- calls the indexer HTTP endpoint
- serializes the returned `Vec<u64>` with `bincode`
- replaces the original raw inputs with the balance vector bytes before dispatch

current limitation:

- the listener identifies historical-average requests by raw input length and shape, not by a dedicated on-chain enum or verifier-aware routing rule

### callback worker

`crates/coordinator/src/callback.rs`:

- pops `ProverResponse` JSON values from `sonar:responses`
- fetches `RequestMetadata` to recover the callback program pubkey
- builds the Anchor `callback` instruction bytes manually
- sends and confirms the Solana transaction with retries

current limitation:

- the callback worker now forwards prover-produced `public_inputs`
- the historical-average path still relies on an MVP verifier helper in the on-chain program rather than a final production verifier rollout
- this means the local historical-average flow works end to end, but it should still be described as MVP-grade rather than production-final

### binary

`bin/coordinator.rs` runs two tasks:

- listener task
- callback worker task

it reads config from `SONAR_CONFIG_PATH` and an optional signing keypair from `SONAR_COORDINATOR_KEYPAIR_PATH`.

## prover

### purpose

the prover consumes jobs from Redis, resolves a registered computation, runs the matching SP1 guest, wraps the proof, and publishes a response.

### registry

`crates/prover/src/registry.rs` registers two computations by ELF hash:

- `fibonacci`
- `historical_avg`

computation ids are the sha256 hash of the guest ELF bytes.

### proving path

`crates/prover/src/lib.rs`:

- resolves the computation id to a named ELF
- loads the ELF from `programs/*/elf/`
- runs either the fibonacci or historical-average SP1 wrapper
- wraps the resulting proof bundle with `wrap_stark_to_groth16`
- returns `(proof, result)`

`crates/prover/src/sp1_wrapper.rs` contains two important execution paths:

- `run_sp1_program` for the fibonacci guest
- `run_historical_avg_program` for a bincode-encoded `Vec<u64>`

for local development the prover can run in mock mode:

- `bin/prover.rs` sets `SP1_PROVER=mock` when `config.prover.mock_prover` is true and the env var is not already set

### queue service

`crates/prover/src/service.rs`:

- `BLPOP`s `sonar:jobs`
- deserializes `ProverJob`
- processes jobs with bounded concurrency using a semaphore
- `RPUSH`es `ProverResponse` into `sonar:responses`

## shared crates

### `crates/common`

shared items include:

- config parsing with `${ENV_VAR}` expansion
- common request, response, and queue types
- tracing initialization and metrics helpers

### `crates/sdk`

`crates/sdk` exists but is minimal today. it only exports a module named `macro` and does not yet provide a full rust client sdk.

### `echo_callback`

`echo_callback` is a test-only Anchor helper program. it accepts the callback CPI and returns immediately. the source explicitly warns that it must not be deployed to mainnet.

## data flow details

```mermaid
sequenceDiagram
    participant client as client program
    participant sonar as sonar program
    participant listener as coordinator listener
    participant idx as indexer http api
    participant redis as Redis
    participant prover as prover service
    participant cb as coordinator callback worker

    client->>sonar: request(request_id, computation_id, inputs, deadline, fee)
    sonar-->>listener: log sonar:request and sonar:inputs
    listener->>sonar: fetch request_metadata account over rpc
    alt historical_avg input shape
        listener->>idx: GET /account_history/:pubkey
        idx-->>listener: Vec<u64> balances
    end
    listener->>redis: RPUSH sonar:jobs ProverJob
    prover->>redis: BLPOP sonar:jobs
    prover->>prover: run SP1 guest and wrap proof
    prover->>redis: RPUSH sonar:responses ProverResponse
    cb->>redis: BLPOP sonar:responses
    cb->>sonar: callback(proof, public_inputs, result)
    sonar-->>client: write result_account and invoke callback program
```

## security model

### what is currently enforced on-chain

the code enforces these properties on-chain:

- request deadlines and fee presence are checked in `request`
- fees stay escrowed in the request metadata account until `callback` or `refund`
- a callback can only succeed once per pending request
- proof verification happens inside the program for known computation ids
- the callback target must be executable
- refunds can only be claimed by the original payer after deadline expiry

### what is currently trusted off-chain

these areas still rely on operator trust or unfinished wiring:

- service liveness depends on the coordinator, prover, Redis, PostgreSQL, and Solana RPC availability
- historical-average inputs are fetched by the coordinator from the indexer and are not committed on-chain before proof verification
- the historical-average verifier path in the on-chain program is still MVP-specific rather than a finished production verifier rollout
- there is no staking, slashing, prover admission policy, or multi-prover consensus mechanism in the codebase yet

### practical consequence

sonar currently provides strong on-chain correctness guarantees for the demo Groth16 verifier path used in program tests. in addition, the repository now demonstrates a working local historical-average MVP path from request to callback, but that path is not yet a finished production verifier rollout.

## current performance-related settings

these are the concrete limits or defaults present in code and config today:

| setting                           | value         | source                         |
| --------------------------------- | ------------- | ------------------------------ |
| max result bytes                  | `10_000`      | `program/src/lib.rs`           |
| coordinator callback timeout      | `30` seconds  | `config/default.toml`          |
| coordinator max concurrent jobs   | `8`           | `config/default.toml`          |
| prover poll timeout               | `1` second    | `crates/prover/src/service.rs` |
| callback worker redis pop timeout | `2.0` seconds | `bin/coordinator.rs`           |
| indexer concurrency               | `4`           | `config/default.toml`          |
| indexer http port                 | `8080`        | `config/default.toml`          |
| metrics port                      | `9090`        | `config/default.toml`          |

these are operational defaults, not guaranteed benchmarks.

## limitations and future work

the following gaps are visible directly in the repository:

- historical-average callback verification is still MVP-specific and not yet presented as a final production verifier design
- `config/devnet.toml` predates the phase 6 config shape and is missing `indexer.http_port` and `coordinator.indexer_url`
- `tests/integration.rs` and `tests/property.rs` are placeholders for later phases
- `crates/sdk` is still a stub and there is no checked-in ts sdk or cli
- no production deployment manifests are present for Redis, PostgreSQL, or the service processes

until those gaps are closed, the repository is best understood as a strong local-development and architecture baseline rather than a finished production network.
