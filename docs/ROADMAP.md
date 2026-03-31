# roadmap

## how to read this file

this roadmap tracks implementation status against the code that is actually present in the repository.

status values:

- `[x]` complete in code and covered by existing tests or ci checks
- `[-]` partially implemented or scaffolded, but missing one or more parts of the original goal
- `[ ]` not implemented yet

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
- the prover registry resolves `historical_avg`
- the indexer serves historical balances over HTTP
- the coordinator enriches historical-average inputs through the indexer
- the prover can execute the historical-average guest

missing pieces that keep 6.1 from complete:

- the on-chain verifier registry still only knows `DEMO_COMPUTATION_ID`
- the callback worker still passes empty `public_inputs`
- there is no checked-in historical-average e2e test proving request to callback on-chain
- there is no checked-in example client program for historical average requests

### phase 7 - testing and hardening

- `[ ]` 7.1 property tests for zk and math
- `[ ]` 7.2 chaos and fork testing

current state:

- `tests/property.rs` exists as a placeholder only
- `tests/integration.rs` exists as a placeholder only

### phase 8 - performance optimisation

- `[ ]` 8.1 benchmarking and optimisation

### phase 9 - developer experience

- `[ ]` 9.1 full sdk, cli, and developer tooling

current state:

- `crates/sdk` is still minimal
- there is no checked-in cli crate
- documentation now exists, but the sdk and cli portions of the phase are not implemented

### phase 10 - testnet and mainnet readiness

- `[ ]` 10.1 deployment hardening and network rollout

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

1. register non-demo verifier keys on-chain for the historical-average computation id
2. thread real `public_inputs` through prover output, coordinator callback submission, and on-chain verification
3. add a real historical-average end-to-end test
4. update `config/devnet.toml` to match the current config struct
5. replace `tests/property.rs` and `tests/integration.rs` placeholders with real phase 7 coverage

## tested status snapshot

the repository currently has evidence of these verified workflows:

- ci runs formatting, clippy, Rust tests, cargo audit, cargo deny, Anchor build, and Anchor tests
- the program request, callback, and refund lifecycle is exercised in `program/tests/sonar.ts`
- the indexer has database and HTTP handler tests
- the prover has registry, wrapper, and service tests
- the coordinator has parser, queue, and callback instruction tests

the repository does not yet contain a checked-in proof that the historical-average template completes a real on-chain callback verification path.
