# Contributing

Thanks for working on Sonar.

This repo has moved beyond pure experimentation: changes should now preserve correctness, keep docs aligned with the code, and leave validation coverage better than you found it when practical.

## Prerequisites

Install these before doing substantive work:

- Rust stable
- Node.js 20+
- Solana CLI 3.0.13
- Anchor CLI 0.32.1
- Docker for integration/e2e flows
- PostgreSQL and Redis if you want to run the off-chain stack manually

Then install dependencies:

```bash
npm install
```

## Recommended local workflow

For most changes, work through this sequence:

```bash
cargo fmt
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --skip integration
```

If you touch the Anchor program or consumer programs, also run:

```bash
bash scripts/anchor-test.sh
```

If you touch orchestration around the historical-average flow, also run:

```bash
cargo build --bins
cargo build -p sonar-indexer --lib
cargo test --test e2e_historical_avg -- --ignored --nocapture
```

If you touch GitHub Actions or release automation, also run the relevant local workflow through the repo wrapper when possible:

```bash
cp .secrets.example .secrets
scripts/local-ci.sh -W .github/workflows/ci.yml -j check
scripts/local-ci.sh -W .github/workflows/security.yml -j audit
```

The repository's `.actrc` maps `ubuntu-latest` to `catthehacker/ubuntu:full-latest` so local CI is closer to the hosted GitHub runner environment. If your machine requires registry authentication to pull that runner image, run `docker login` first.

If you touch devnet deployment or the prod-oriented service topology, also sanity-check the relevant operational entrypoints:

```bash
bash -n scripts/deploy-devnet.sh
docker compose -f docker-compose.prod.yml config
```

If you touch hot paths in the coordinator or prover, consider running:

```bash
cargo bench -p sonar-coordinator
cargo bench -p sonar-prover
```

## Pre-commit hooks

The repository includes a `.pre-commit-config.yaml` with:

- `cargo fmt`
- `clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo deny check`
- `cargo audit`
- Prettier for Markdown, JSON, YAML, and YML

Using pre-commit locally is recommended, especially for doc-heavy changes.

## Contribution standards

### Keep changes focused

- fix root causes when possible
- avoid unrelated cleanup in the same change
- preserve existing public interfaces unless the change explicitly requires otherwise

### Keep docs synchronized

If you change any of the following, update the docs in the same PR when needed:

- architecture or service boundaries
- repo layout or developer workflow
- roadmap status
- verifier/artifact flow
- config expectations
- deployment or observability workflows

The canonical permanent docs are:

- `README.md`
- `docs/SSOT.md`
- `docs/ROADMAP.md`
- `docs/ARCHITECTURE.md`
- `docs/PROD_TARGET.md`
- `docs/CONTRIBUTING.md`
- `SECURITY.md`

If you touch a Mermaid diagram, make sure the surrounding prose still matches the implementation.

### Prefer validation over assumption

- add or update tests when the code path already has a natural test surface
- do not claim a flow works unless you actually ran the relevant validation
- if a broader suite is broken for unrelated reasons, call that out clearly in your PR notes

## Common change buckets

### On-chain program changes

Expect to update some combination of:

- Anchor tests in `program/tests/`
- Rust tests in `program/`
- architecture docs
- SSOT/roadmap entries if capability status changed

### Off-chain service changes

Expect to update some combination of:

- coordinator/prover/indexer unit tests
- e2e historical-average flow
- config docs
- operations-focused sections of the README and architecture docs

### Deployment and ops changes

Expect to update some combination of:

- `docker-compose.prod.yml`
- `docker/prometheus/prometheus.yml`
- README operational workflow sections
- SSOT and architecture notes about topology boundaries
- roadmap / prod-target notes when the capability level changes

### Verifier or proving changes

Expect to update some combination of:

- artifact export flows
- CLI behavior
- roadmap/prod-target docs
- benchmark coverage if the change affects hot paths

## Pull request checklist

Before asking for review, make sure you can honestly say:

- the change is scoped and explained clearly
- relevant tests/build steps were run
- docs were updated if behavior or workflow changed
- no secrets or local-only config leaked into the diff
- any known gaps or follow-ups are called out explicitly

## Reporting unclear areas

If you find contradictions between code and docs, fix them when they are obviously local to your change. If the mismatch is broader, leave the repo in a better state and call out the remaining inconsistency in your handoff.
