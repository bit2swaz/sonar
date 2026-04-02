# Historical Average MVP Demo Plan

This version is optimized for a short no-audio screen recording for X/Twitter.

Goal: show that Sonar can run a historical-average request end to end across Solana, PostgreSQL, Redis, the coordinator, the prover, and the final on-chain result account.

## Recommended format

- Length: 45 to 90 seconds
- Layout: 2 panes max
  - left: main command terminal
  - right: live logs terminal
- Font: large enough to read on mobile
- Keep every important proof point on screen for 2 to 4 seconds
- Add short text overlays in post rather than relying on terminal commentary

## What this demo should prove visually

- the local Sonar stack starts successfully
- the indexer serves historical balances
- the coordinator dispatches a prover job
- the prover completes the historical-average computation
- the callback lands and writes the result
- `historical_avg_result` equals `expected_avg`

## What not to imply

- Do not present this as a finished production verifier system.
- Present it as the current MVP historical-average flow working end to end locally.

## Best recording flow

### Option A — best for a clean social clip

Use the automated verifier first off-camera to make sure the environment is healthy:

```bash
./scripts/verify-demo.sh
```

Then record the manual flow below.

### Option B — fastest confidence check before posting

If you want a backup clip or a self-check run:

```bash
./scripts/demo-historical-avg.sh --no-pause demo
```

## Shot list

### Shot 1 — opening frame

On-screen text:

> Sonar MVP: historical-average request → proof → callback → on-chain result

Keep this on screen for 2 seconds before typing.

### Shot 2 — start the stack

Main terminal:

```bash
./scripts/demo-historical-avg.sh start
```

What to leave visible when it finishes:
- `observed_pubkey`
- `from_slot`
- `to_slot`
- `expected_avg`

Suggested overlay:

> 1. Boot local Sonar stack and seed account history

### Shot 3 — open live logs

Second terminal:

```bash
./scripts/demo-historical-avg.sh logs
```

Keep this running for the rest of the clip.

Suggested overlay:

> 2. Watch request, prover, and callback logs live

### Shot 4 — show indexed history

Main terminal:

```bash
./scripts/demo-historical-avg.sh status
curl -s "http://127.0.0.1:${INDEXER_HTTP_PORT:-18080}/account_history/$(python3 - <<'PY'
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
```

Pause briefly on the returned lamport list.

Suggested overlay:

> 3. Indexer returns the historical balances used as prover input

### Shot 5 — submit the request

Main terminal:

```bash
./scripts/demo-historical-avg.sh request
```

Leave these visible:
- `request_id_hex`
- `request_metadata`
- `result_account`

At the same time, let the logs terminal show:
- `sonar:request:`
- coordinator activity
- prover activity
- callback submission

Suggested overlay:

> 4. Coordinator picks up the request and dispatches the prover job

### Shot 6 — show the payoff moment

Main terminal:

```bash
./scripts/demo-historical-avg.sh result
```

This is the most important shot in the whole video.

Keep the matching lines centered on screen:
- `historical_avg_result=...`
- `expected_avg=...`

Suggested overlay:

> 5. Callback completes and the on-chain result matches the expected average

Hold this frame for 3 to 4 seconds.

### Shot 7 — final state check

Main terminal:

```bash
./scripts/demo-historical-avg.sh status
```

Optional extra commands if the clip still has room:

```bash
solana --url http://127.0.0.1:${RPC_PORT:-18899} account "$(python3 - <<'PY'
import json
with open('.demo/historical-avg/state.json', 'r', encoding='utf-8') as fh:
   state = json.load(fh)
print(state['request_metadata'])
PY
)"
solana --url http://127.0.0.1:${RPC_PORT:-18899} account "$(python3 - <<'PY'
import json
with open('.demo/historical-avg/state.json', 'r', encoding='utf-8') as fh:
   state = json.load(fh)
print(state['result_account'])
PY
)"
```

Suggested overlay:

> Final Sonar PDAs are written on-chain

### Shot 8 — clean teardown

Main terminal:

```bash
./scripts/demo-historical-avg.sh stop
```

Suggested overlay:

> Clean shutdown — safe to rerun locally

## Fastest version for social media

If you want the shortest possible cut, keep only these shots:

1. `start`
2. `request`
3. `result`
4. `stop`

That is enough for a compact 45 to 60 second post if the logs terminal is visible throughout.

## Suggested overlay text sequence

Use these as captions in post-production:

1. `Boot local Sonar stack`
2. `Seed historical account balances`
3. `Request emitted on Solana`
4. `Coordinator dispatches prover job`
5. `Prover computes historical average`
6. `Callback writes result on-chain`
7. `Result matches expected average`

## Recording tips

- Run once before recording so Docker images and Rust artifacts are already warm.
- Close noisy notifications and browser tabs.
- Use a dark theme and a large terminal font.
- Avoid resizing panes during the take.
- The demo scripts now default to safer high ports: Postgres `15432`, Redis `16379`, indexer `18080`, RPC `18899`, faucet `19900`, dynamic range `20000-20030`.
- If you still need different ports, export alternate ports before recording and keep them consistent across both terminals.

## Final caption idea for the post

> Sonar MVP: a historical-average request flows from Solana logs → indexer enrichment → prover job → callback → final on-chain result.