# roadmap

## how to read this file

this roadmap is the canonical transition plan from the current Sonar MVP to the production target described in [docs/PROD_TARGET.md](docs/PROD_TARGET.md).

it starts from the code that exists today, but phases 6 and beyond are forward-looking execution work rather than claims that those features are already implemented.

for the strictly factual "what exists now" view, use [docs/SSOT.md](docs/SSOT.md).

this file is where the current repository state hands off into the next production path.

status values:

- `[x]` complete in code and covered by existing tests or ci checks
- `[-]` partially implemented or scaffolded, but missing one or more parts of the original goal
- `[ ]` not implemented yet

## document role

phases 0 through 5 describe the completed MVP foundation already present in the repository.

phase 6 captures the active production-verifier pivot now that the local historical-average flow exists.

phases 7 and beyond define the canonical next execution path for turning the MVP into a production-grade coprocessor stack.

## phase summary

| phase | title | status | notes |
| --- | --- | --- | --- |
| 0 | project hygiene | `[x]` | ci, hooks, secret scanning, audit, deny |
| 1 | foundation | `[x]` | workspace, shared config, shared types, observability helpers |
| 2 | Solana program | `[x]` | request, callback, refund, verifier demo path, Anchor tests |
| 3 | off-chain prover | `[x]` | SP1 execution, registry, service loop, proof wrapping |
| 4 | state indexer | `[x]` | Geyser plugin export, PostgreSQL storage, migrations, queries |
| 5 | coordinator and queue | `[x]` | websocket listener, Redis dispatch, callback worker |
| 6 | end-to-end mvp | `[-]` | historical-average pipeline exists off-chain but is not fully verified on-chain |
| 7 | testing and hardening | `[ ]` | placeholders only |
| 8 | performance optimisation | `[ ]` | not implemented |
| 9 | developer experience | `[ ]` | docs are improving, sdk and cli are still incomplete |
| 10 | testnet and mainnet readiness | `[ ]` | no deployment automation beyond current scripts |
| 11 | token and staking | `[ ]` | not implemented |
| 12 | decentralisation and cross-chain | `[ ]` | not implemented |
| 13 | open source and grants | `[ ]` | not implemented |

## mini-phase status

### phase 0 - project hygiene

- `[x]` 0.1 git, ci, and linting baseline
- `[x]` 0.2 secret scanning and dependency pinning

evidence:

- `.github/workflows/ci.yml`
- `.github/workflows/security.yml`
- `.pre-commit-config.yaml`
- `.gitleaks.toml`
- `deny.toml`
- `rust-toolchain.toml`

### phase 1 - foundation

- `[x]` 1.1 workspace scaffold
- `[x]` 1.2 shared types
- `[x]` 1.3 config loading
- `[x]` 1.4 observability helpers

evidence:

- workspace members in `Cargo.toml`
- `crates/common/src/types.rs`
- `crates/common/src/config.rs`
- `crates/common/src/metrics.rs`

### phase 2 - Solana program

- `[x]` 2.1 request path
- `[x]` 2.2 callback and refund path
- `[x]` 2.3 integration tests against local validator

evidence:

- `program/src/lib.rs`
- `program/src/verifier_registry.rs`
- `program/tests/sonar.ts`
- `echo_callback/src/lib.rs`

notes:

- the verified path is still the built-in demo verifier, not the historical-average computation

### phase 3 - off-chain prover

- `[x]` 3.1 SP1 integration and proof wrapping
- `[x]` 3.2 Redis-backed prover service

evidence:

- `crates/prover/src/lib.rs`
- `crates/prover/src/sp1_wrapper.rs`
- `crates/prover/src/groth16_wrapper.rs`
- `crates/prover/src/registry.rs`
- `crates/prover/src/service.rs`

### phase 4 - state indexer

- `[x]` 4.1 Geyser plugin skeleton
- `[x]` 4.2 PostgreSQL-backed account history

evidence:

- `crates/indexer/src/lib.rs`
- `crates/indexer/src/geyser_plugin.rs`
- `crates/indexer/src/db.rs`
- `crates/indexer/migrations/202603310001_init_account_history.sql`

### phase 5 - coordinator and queue

- `[x]` 5.1 listener, dispatch, and callback worker

evidence:

- `crates/coordinator/src/listener.rs`
- `crates/coordinator/src/dispatcher.rs`
- `crates/coordinator/src/callback.rs`
- `bin/coordinator.rs`

### phase 6 - end-to-end mvp

- `[-]` 6.1 historical average template

implemented pieces:

- `programs/historical_avg/` guest crate exists
- `programs/historical_avg_client/` example client program exists in the workspace
- the prover registry resolves `historical_avg`
- the indexer serves historical balances over HTTP
- the coordinator enriches historical-average inputs through the indexer
- the prover can execute the historical-average guest
- the repository now includes checked-in local e2e coverage for the historical-average flow

missing pieces that keep 6.1 from complete:

- the on-chain historical-average path still relies on an MVP verifier helper rather than a fully separate production verifier rollout
- the historical-average path is covered locally, but not yet presented as a hardened production-ready verification path

next execution path for phase 6:

- `[ ]` 6.2 extract proving artifacts and computation metadata in a repeatable way
- `[ ]` 6.3 add an on-chain verifier-registry path or equivalent production-grade verifier mapping keyed by `computation_id`
- `[ ]` 6.4 harden coordinator callback proof formatting for the production verifier flow
- `[ ]` 6.5 replace the MVP helper with real on-chain cryptographic verification for historical-average callbacks
- `[ ]` 6.6 add explicit negative tests for mutated proofs and mutated public inputs

### phase 7 - testing and hardening

- `[ ]` 7.1 property tests for zk and math
- `[ ]` 7.2 chaos and fork testing
- `[ ]` 7.3 refund and deadline edge-case hardening

current state:

- `tests/property.rs` exists as a placeholder only
- `tests/integration.rs` exists as a placeholder only

target outcome for this phase:

- prover and guest logic handle malformed and extreme inputs safely
- queue workers fail gracefully on corrupted payloads
- refund timing rules are test-covered at exact slot boundaries

### phase 8 - performance optimisation

- `[ ]` 8.1 benchmarking and optimisation
- `[ ]` 8.2 indexing and callback-path performance improvements

target outcome for this phase:

- better historical lookup performance
- better callback inclusion under realistic network conditions
- measurement-driven tuning rather than ad hoc changes

### phase 9 - developer experience

- `[ ]` 9.1 full sdk, cli, and developer tooling

target outcome for this phase:

- a clearer Rust SDK story for request/callback integration
- better automation for build artifacts and verifier-registration workflows
- lower-friction developer onboarding from guest program to deployed verifier path

current state:

- `crates/sdk` is still minimal
- there is no checked-in cli crate
- documentation now exists, but the sdk and cli portions of the phase are not implemented

### phase 10 - testnet and mainnet readiness

- `[ ]` 10.1 deployment hardening and network rollout

target outcome for this phase:

- config parity across environments
- deployable infrastructure definitions for off-chain services
- a cleaner path from local MVP validation to persistent hosted environments

### phase 11 - token and staking

- `[ ]` 11.1 token and staking program

### phase 12 - decentralisation and cross-chain

- `[ ]` 12.1 permissionless provers and recursion
- `[ ]` 12.2 cross-chain verification

### phase 13 - open source and grants

- `[ ]` 13.1 comprehensive public docs and examples
- `[ ]` 13.2 grants and ecosystem material

## current next steps

the highest-value next steps visible from the codebase are:

1. replace the current historical-average MVP verifier path with a distinct production-grade verifier rollout
2. update `config/devnet.toml` to match the current config struct
3. replace `tests/property.rs` and `tests/integration.rs` placeholders with real phase 7 coverage
4. expand production-facing docs and deployment guidance around the new e2e/demo coverage
5. continue reducing config and env-var legacy surface area

## production-path checkpoints

the next canonical transition from the current MVP into the production target is:

1. finish the production historical-average verifier path on-chain
2. harden proof formatting and callback security checks
3. add the missing hardening test layers
4. sync the config and deployment model with the real runtime shape
5. improve SDK and tooling ergonomics around the proving flow

## tested status snapshot

the repository currently has evidence of these verified workflows:

- ci runs formatting, clippy, Rust tests, cargo audit, cargo deny, Anchor build, and Anchor tests
- ci now also runs the ignored Rust historical-average e2e test and automated demo verification
- the program request, callback, and refund lifecycle is exercised in `program/tests/sonar.ts`
- the indexer has database and HTTP handler tests
- the prover has registry, wrapper, and service tests
- the coordinator has parser, queue, and callback instruction tests

the repository now contains checked-in local proof that the historical-average template completes the repository's current MVP callback flow, but it does not yet represent a finished production-grade verification path.
