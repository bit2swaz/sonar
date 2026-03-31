# Historical Average Demo Video Plan

This walkthrough assumes you are in the repository root and have Docker, the Solana CLI, and Anchor installed.

1. terminal 1: start the local demo stack
   $ ./scripts/demo-historical-avg.sh start

   Commentary:
   - Let the audience see the stack build local artifacts, start `solana-test-validator`, deploy `sonar` and `historical_avg_client`, start PostgreSQL + Redis, and then launch `sonar-indexer`, `sonar-prover`, and `sonar-coordinator`.
   - Point out the printed `observed_pubkey`, seeded slot range, and `expected_avg`.

2. terminal 2: show live validator and service logs
   $ ./scripts/demo-historical-avg.sh logs

   Commentary:
   - Keep this terminal visible for the rest of the demo.
   - When the request is sent, highlight:
     - `sonar:request:` and `sonar:inputs:` in the validator log
     - the coordinator detecting the request and dispatching a `ProverJob`
     - the prover log line showing the historical-average proof job being processed
     - the coordinator log line showing the callback transaction being submitted

3. terminal 3: confirm the seeded account history and expected average
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
   - Briefly show the raw lamport history returned by the indexer and explain that this is the data the coordinator converts into prover inputs.

4. terminal 3: submit the manual historical-average request
   $ ./scripts/demo-historical-avg.sh request

   Commentary:
   - Call out the printed `request_id_hex`, `request_metadata`, `result_account`, and `callback_state` PDAs.
   - In terminal 2, show:
     - the coordinator listener reacting to the request log
     - the prover handling the job and generating the proof
     - the callback worker submitting the on-chain callback transaction
     - the validator confirming the callback instruction

5. terminal 3: wait for the callback and print the decoded result
   $ ./scripts/demo-historical-avg.sh result

   Commentary:
   - This prints both `historical_avg_result` and `expected_avg`.
   - Emphasize that the callback state PDA is now set by the client program, proving the full request → indexer enrichment → prover → callback flow completed.

6. terminal 3: inspect the final Sonar PDAs
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
   - Explain that the request metadata PDA is completed and the result PDA now stores the historical average returned through the verified callback.

7. terminal 1: shut everything down cleanly
   $ ./scripts/demo-historical-avg.sh stop

   Commentary:
   - Mention that the demo script cleans up the validator, services, and Docker containers so it is safe to rerun.

## Log callouts

- Proof generation: [scripts/demo-historical-avg.sh](scripts/demo-historical-avg.sh) writes prover logs to `.demo/historical-avg/logs/prover.log`.
- Callback submission and on-chain verification: the coordinator writes to `.demo/historical-avg/logs/coordinator.log`, while the validator writes program logs to `.demo/historical-avg/logs/validator.log`.
- If you want a single command for a live take, use:

  $ ./scripts/demo-historical-avg.sh demo

  That runs the entire stack, submits the request, waits for the callback, prints the result, and pauses for teardown.