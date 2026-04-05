# Sonar Roadmap

This roadmap separates what is already done from the work still required to move Sonar from a solid prototype into a production-grade coprocessor platform.

## Summary

Sonar has already crossed the threshold from “design sketch” to “working system”:

- the on-chain request/callback/refund loop exists
- dynamic verifier registration exists
- the coordinator/prover/indexer pipeline exists
- `historical_avg` works end-to-end
- the repo ships a CPI SDK and a verifier-registration CLI
- hardening, benchmarks, and security automation are now in place

The next phase is less about inventing new primitives and more about productionizing the ones that now exist.

## Status table

| Area                              | Status   | Notes                                                                   |
| --------------------------------- | -------- | ----------------------------------------------------------------------- |
| Core on-chain request lifecycle   | Complete | `request`, `callback`, and `refund` are implemented                     |
| Dynamic verifier registration     | Complete | `register_verifier` plus CLI/artifact flow are in-tree                  |
| Off-chain orchestration           | Complete | Coordinator, Redis queues, prover, and callback worker exist            |
| Indexing and enrichment           | Complete | Geyser plugin + Postgres + account-history API exist                    |
| Historical-average vertical slice | Complete | End-to-end test and CI path exist                                       |
| Developer SDK                     | Complete | `crates/sdk` provides a real CPI helper                                 |
| Developer CLI                     | Complete | `crates/cli` provides verifier registration                             |
| Hardening and benchmarks          | Complete | Panic-path cleanup, failure tests, Criterion benches, CI/security gates |
| Documentation refresh             | Complete | README + docs now aligned to current repo state                         |
| Production operations             | Planned  | SLOs, runbooks, staged rollout, on-call practices                       |
| Production economics              | Planned  | fee policy, proving cost controls, congestion handling                  |
| Governance and security posture   | Planned  | verifier authority lifecycle, audit posture, release process            |
| External productization           | Planned  | richer API/SDK surface, more computations, better operator UX           |

## What is done

### 1. Core protocol primitives

The repo already has the essential contract surface for a coprocessor:

- request state
- result state
- escrowed fees
- callback execution
- refunds
- registered verifier state

### 2. End-to-end orchestration

The repo also has a functioning off-chain pipeline:

- log ingestion from Solana
- job dispatch through Redis
- proof generation and Groth16 wrapping
- callback submission back to the program
- indexed historical account data for computation enrichment

### 3. Developer experience baseline

The repo now ships the minimum set of tools downstream integrators need:

- CPI helper crate for in-program use
- CLI for verifier registration
- artifact export path for prover outputs
- example consumer and callback programs

### 4. Hardening baseline

Recent work added the first real hardening layer:

- failure-path tests
- cleanup of production panic paths
- Criterion benchmarks for coordinator and prover hot paths
- explicit CI coverage for Anchor and e2e flows
- supply-chain and secret-scanning automation

## What comes next

### Production operations

Highest-value remaining work:

- define service-level objectives for coordinator, prover, and indexer
- document deploy, rollback, and incident-response runbooks
- make recovery and replay workflows routine rather than ad hoc
- add stronger environment separation between local/devnet/staging/production

### Production economics

Sonar needs a clearer resource model before production use:

- fee policy tied to proof cost and queue pressure
- backpressure and admission control
- capacity planning for proving throughput and callback latency
- clearer payout and reimbursement semantics for failed/offline cases

### Governance and trust model

Verifier registration is present, but governance is still thin:

- authority rotation and revocation strategy
- explicit verifier lifecycle states
- release policy for computation artifacts and verifying keys
- pre-mainnet external review and audit preparation

### Broader product surface

To become a general coprocessor rather than a single vertical slice, Sonar still needs:

- more first-class computations
- more ergonomic client libraries and examples
- a richer operator API than the current account-history endpoint
- clearer multi-tenant and multi-environment operating patterns

## Exit criteria for “production candidate”

Sonar should not be treated as production-ready until all of the following are true:

- verifier lifecycle and authority management are operationally safe
- proving economics are explicit and observable
- recovery and rollback procedures are documented and rehearsed
- observability covers queue health, callback latency, proof failures, and data freshness
- the repo has undergone external security review appropriate for the intended deployment
