# Security

## Current posture

Sonar is an actively developed prototype and should currently be treated as pre-production software. The repository contains meaningful hardening work and automated security checks, but it has not yet reached the bar implied by a mainnet-grade deployment.

If you are evaluating or extending Sonar, assume:

- correctness is a primary design goal
- operational maturity is still in progress
- verifier governance and release processes are not fully formalized yet

## Reporting vulnerabilities

Please do not open a public issue for a suspected security vulnerability.

Instead:

1. contact the maintainers through a private disclosure channel if you already have one
2. if a private channel is not established, request one before sharing exploit details publicly
3. include clear reproduction steps, affected components, impact, and any proof-of-concept artifacts

Until the repository publishes a more formal disclosure program, treat coordinated private disclosure as the expected path.

## Security-relevant areas in this repo

### On-chain program

High-sensitivity surfaces include:

- verifier registration
- proof verification
- callback invocation
- escrowed fee movement
- refund conditions and account constraints

### Off-chain services

Security-sensitive off-chain areas include:

- coordinator transaction construction and callback submission
- prover artifact integrity and computation resolution
- indexer data freshness and query correctness
- queue integrity between coordinator and prover

### Supply chain and secrets

The repo also treats these as security concerns:

- dependency vulnerabilities
- license/compliance drift
- leaked credentials or RPC keys
- unsafe local configuration being committed accidentally

## Automated controls currently in place

### CI

The CI workflow currently runs:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- workspace tests
- Anchor build
- Anchor tests
- historical-average end-to-end coverage

### Security workflow

The dedicated security workflow currently runs:

- `cargo audit`
- `cargo deny check`
- `gitleaks`

Those checks can also be rehearsed locally through `scripts/local-ci.sh` once `.secrets` exists. Some local `act` environments may also require `docker login` before the configured runner image can be pulled.

### Local hooks

The pre-commit configuration includes:

- Rust formatting
- clippy with warnings denied
- `cargo deny`
- `cargo audit`
- Prettier for docs and config files

## Secure development guidance

- prefer fixing panic paths and unchecked assumptions in long-running services
- validate artifact integrity whenever verifier material crosses a trust boundary
- keep secrets in environment variables or local config, never in committed files
- keep `.secrets` local-only and start from `.secrets.example` when using local CI
- update docs when a trust boundary or operator workflow changes
- favor fail-closed behavior for proof, verifier, and callback mismatches

## Out of scope for current guarantees

The repo does not currently claim:

- a formal bug bounty program
- public response-time SLAs for disclosures
- production deployment guarantees
- completed external audit coverage for every component

## Practical expectation

Responsible contributors should assume that any change touching proof verification, verifier material, callback execution, escrowed funds, queue semantics, or artifact handling deserves extra review and stronger-than-average validation.
