# Historical Average Demo Video Plan

This script is for a polished walkthrough of the historical-average flow that is actually implemented in the repository today.

What this demo proves on camera:
- the local stack boots cleanly
- the indexer records account history in PostgreSQL
- the coordinator enriches the request via the indexer HTTP API
- the prover processes the historical-average job
- the callback transaction writes the final Sonar result account
- the printed `historical_avg_result` matches the seeded `expected_avg`

What it does not claim:
- a production-final historical-average verifier path beyond the current MVP checks in the on-chain program

## Recommended setup

- Terminal 1: stack control
- Terminal 2: live logs
- Terminal 3: commands and narration

If your machine already uses ports such as `5432`, `6379`, `8080`, or `8899`, export alternate ports first and reuse them in every command.

## Pre-flight line

Say this before you start:

> “This demo shows Sonar’s current verified local flow for historical-average requests: request creation, indexed balance lookup, prover execution, callback submission, and final result persistence on-chain.”

## Demo walkthrough

1. Terminal 1 — start the local stack

   $ ./scripts/demo-historical-avg.sh start

   Commentary:
   - Let the audience see the script build local Rust artifacts, start PostgreSQL and Redis, boot `solana-test-validator`, and launch `sonar-indexer`, `sonar-prover`, and `sonar-coordinator`.
   - Call out the printed `observed_pubkey`, `from_slot`, `to_slot`, and `expected_avg`.
   - Mention that the script also seeds a real account-history window so the indexer has concrete data to serve.

2. Terminal 2 — tail validator and service logs

   $ ./scripts/demo-historical-avg.sh logs

   Commentary:
   - Keep this terminal visible throughout the request and callback sequence.
   - Tell the audience what to watch for:
     - validator logs with `sonar:request:` and `sonar:inputs:`
     - coordinator logs showing request detection and dispatch
     - prover logs showing the historical-average job being executed
     - coordinator logs showing callback submission back to Solana

3. Terminal 3 — show seeded state and raw indexed history

   $ ./scripts/demo-historical-avg.sh status
   $ curl -s "http://127.0.0.1:8080/account_history/$(python3 - <<'PY'
import json
with open('.demo/historical-avg/state.json', 'r', encoding='utf-8') as fh:
    state = json.load(fh)
print(state['observed_pubkey'])
PY
)?from_slot=$(python3 - <<'PY'
import json
with open('.demo/historical-avg/state.json', 'r', encoding='utf-8') as fh:
    state = json.load(fh)
print(state['from_slot'])
PY
)&to_slot=$(python3 - <<'PY'
import json
with open('.demo/historical-avg/state.json', 'r', encoding='utf-8') as fh:
    state = json.load(fh)
print(state['to_slot'])
PY
)"

   Commentary:
   - Explain that the indexer is returning the lamport history that the coordinator will convert into prover inputs.
   - Emphasize that this is the off-chain enrichment step specific to the historical-average computation.

4. Terminal 3 — submit the historical-average request

   $ ./scripts/demo-historical-avg.sh request

   Commentary:
   - Call out the printed `request_id_hex`, `request_metadata`, and `result_account` addresses.
   - Switch attention to Terminal 2 and narrate the flow in order:
     - the on-chain request emits logs
     - the coordinator notices the request and fetches metadata
     - the coordinator queries the indexer for historical balances
     - the prover processes the job
     - the coordinator submits the callback transaction

5. Terminal 3 — wait for the callback and show the result

   $ ./scripts/demo-historical-avg.sh result

   Commentary:
   - This prints both `historical_avg_result` and `expected_avg`.
   - Pause here and say explicitly that the end-to-end flow succeeded because the result written on-chain matches the seeded expected average.
   - Use this as the main “proof point” moment in the video.

6. Terminal 3 — inspect final Sonar state

   $ ./scripts/demo-historical-avg.sh status
   $ solana --url http://127.0.0.1:8899 account "$(python3 - <<'PY'
import json
with open('.demo/historical-avg/state.json', 'r', encoding='utf-8') as fh:
    state = json.load(fh)
print(state['request_metadata'])
PY
)"
   $ solana --url http://127.0.0.1:8899 account "$(python3 - <<'PY'
import json
with open('.demo/historical-avg/state.json', 'r', encoding='utf-8') as fh:
    state = json.load(fh)
print(state['result_account'])
PY
)"

   Commentary:
   - Explain that the request metadata account is no longer pending and the result account now contains the computed historical average.
   - Be precise: this demonstrates the repository’s current MVP historical-average callback path, not a finished production verifier story.

7. Terminal 1 — shut the stack down

   $ ./scripts/demo-historical-avg.sh stop

   Commentary:
   - Mention that the script tears down validator, services, and Docker containers, so the flow is safe to rerun.

## One-command options

- For a live recorded take that still pauses before teardown:

  $ ./scripts/demo-historical-avg.sh demo

  This runs the full flow, prints the result, and waits for enter before cleanup.

- For a non-interactive confidence check before recording:

  $ ./scripts/verify-demo.sh

  This runs the same flow automatically and exits with `VERIFICATION PASSED` only when the printed `historical_avg_result` matches `expected_avg`.

## Log callouts to capture on screen

- Indexer history lookup and HTTP serving: `.demo/historical-avg/logs/indexer.log`
- Proof job execution: `.demo/historical-avg/logs/prover.log`
- Callback submission: `.demo/historical-avg/logs/coordinator.log`
- On-chain request and callback program logs: `.demo/historical-avg/logs/validator.log`

## Suggested closing line

> “Today’s Sonar repo already demonstrates a real local request-to-result flow for historical-average jobs across Solana, PostgreSQL, Redis, the coordinator, and the prover. The remaining work is turning this MVP verifier path into a fuller production-grade on-chain verification story.”