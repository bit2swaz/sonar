# Security Policy

## Reporting Vulnerabilities

Do **not** open a public GitHub issue for security vulnerabilities.

Please disclose vulnerabilities responsibly by emailing the maintainers directly.
Include as much detail as possible: affected component, reproduction steps, and potential impact.

We aim to acknowledge reports within 48 hours and issue a patch within 14 days for critical findings.

## Known Risk Areas

- **`PRIVATE_KEY` env var:** Never log, never commit, rotate immediately if exposed.  
  If you suspect a key was leaked, rotate it before doing anything else.
- **RPC endpoints & API keys:** Treat all API keys (Helius, RPC providers) as secrets.  
  Use GitHub Secrets for CI; use a secrets manager (e.g., HashiCorp Vault) in production.
- **Smart contract:** The Sonar program is **not yet audited** — use at your own risk on mainnet.  
  A third-party audit is planned before mainnet launch (Phase 10).
- **ZK proofs:** Verify all proofs **on-chain**; never trust off-chain provers blindly.  
  The on-chain `groth16-solana` syscall is the only authoritative verification step.
- **Dependency supply chain:** `cargo audit` and `cargo deny` run automatically in CI and weekly.  
  Pin dependencies in `Cargo.lock` and review before merging dependency bumps.
