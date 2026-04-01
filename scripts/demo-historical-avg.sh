#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEMO_DIR="${ROOT_DIR}/.demo/historical-avg"
LOG_DIR="${DEMO_DIR}/logs"
LEDGER_DIR="${DEMO_DIR}/ledger"
STATE_JSON="${DEMO_DIR}/state.json"
CONFIG_TOML="${DEMO_DIR}/sonar-demo.toml"
PLUGIN_CONFIG_JSON="${DEMO_DIR}/geyser-plugin.json"
COORDINATOR_KEYPAIR="${DEMO_DIR}/coordinator-keypair.json"
CLIENT_KEYPAIR="${DEMO_DIR}/client-keypair.json"
OBSERVED_KEYPAIR="${DEMO_DIR}/observed-keypair.json"

SONAR_PROGRAM_ID="EE2sQ2VRa1hY3qjPQ1PEwuPZX6dGwTZwHMCumWrGn3sV"
ECHO_CALLBACK_PROGRAM_ID="3RBU9G6Mws9nS8bQPg2cVRbS2v7CgsjAvv2MwmTcmbyA"

POSTGRES_CONTAINER="sonar-demo-postgres"
REDIS_CONTAINER="sonar-demo-redis"

RPC_PORT="${RPC_PORT:-8899}"
WS_PORT="$((RPC_PORT + 1))"
FAUCET_PORT="${FAUCET_PORT:-9900}"
DYNAMIC_PORT_START="${DYNAMIC_PORT_START:-10000}"
DYNAMIC_PORT_END="${DYNAMIC_PORT_END:-10030}"
INDEXER_HTTP_PORT="${INDEXER_HTTP_PORT:-8080}"
POSTGRES_PORT="${POSTGRES_PORT:-5432}"
REDIS_PORT="${REDIS_PORT:-6379}"

export SP1_PROVER="${SP1_PROVER:-mock}"
KEEP_ALIVE_ON_EXIT=0

mkdir -p "${LOG_DIR}" "${LEDGER_DIR}"
cd "${ROOT_DIR}"

need_cmd() {
	command -v "$1" >/dev/null 2>&1 || {
		echo "missing required command: $1" >&2
		exit 1
	}
}

log() {
	echo "[demo] $*"
}

write_state() {
	python3 - "$STATE_JSON" "$@" <<'PY'
import json, sys
path = sys.argv[1]
updates = dict(arg.split('=', 1) for arg in sys.argv[2:])
try:
		with open(path, 'r', encoding='utf-8') as fh:
				state = json.load(fh)
except FileNotFoundError:
		state = {}
for key, value in updates.items():
		state[key] = value
with open(path, 'w', encoding='utf-8') as fh:
		json.dump(state, fh, indent=2, sort_keys=True)
PY
}

read_state() {
	python3 - "$STATE_JSON" "$1" <<'PY'
import json, sys
with open(sys.argv[1], 'r', encoding='utf-8') as fh:
		state = json.load(fh)
value = state.get(sys.argv[2], '')
print(value)
PY
}

cleanup_processes() {
	for name in validator indexer prover coordinator; do
		local pid_file="${DEMO_DIR}/${name}.pid"
		if [[ -f "${pid_file}" ]]; then
			local pid
			pid="$(cat "${pid_file}")"
			if kill -0 "${pid}" >/dev/null 2>&1; then
				log "stopping ${name} (pid ${pid})"
				kill "${pid}" >/dev/null 2>&1 || true
				wait "${pid}" 2>/dev/null || true
			fi
			rm -f "${pid_file}"
		fi
	done
}

cleanup_containers() {
	docker rm -f "${POSTGRES_CONTAINER}" >/dev/null 2>&1 || true
	docker rm -f "${REDIS_CONTAINER}" >/dev/null 2>&1 || true
}

cleanup_all() {
	cleanup_processes
	cleanup_containers
}

on_exit() {
	if [[ "${KEEP_ALIVE_ON_EXIT}" != "1" ]]; then
		cleanup_all
	fi
}

trap on_exit EXIT

write_plugin_config() {
	cat >"${PLUGIN_CONFIG_JSON}" <<EOF
{
	"libpath": "${ROOT_DIR}/target/debug/libsonar_indexer.so",
	"database_url": "postgres://postgres:postgres@127.0.0.1:${POSTGRES_PORT}/postgres",
	"log_level": "info",
	"max_connections": 4,
	"batch_size": 1
}
EOF
}

write_runtime_config() {
	cat >"${CONFIG_TOML}" <<EOF
[network]
rpc_url = "http://127.0.0.1:${RPC_PORT}"
ws_url = "ws://127.0.0.1:${WS_PORT}"
chain_id = "localnet"

[strategy]
min_profit_floor_usd = 0.01
gas_buffer_multiplier = 1.2
max_gas_price_gwei = 1.0

[rpc]
helius_api_key = "dummy"
helius_rpc_url = "http://127.0.0.1:${RPC_PORT}"

[indexer]
geyser_plugin_path = "${ROOT_DIR}/target/debug/libsonar_indexer.so"
database_url = "postgres://postgres:postgres@127.0.0.1:${POSTGRES_PORT}/postgres"
concurrency = 2
http_port = ${INDEXER_HTTP_PORT}

[coordinator]
redis_url = "redis://127.0.0.1:${REDIS_PORT}"
callback_timeout_seconds = 30
max_concurrent_jobs = 4
indexer_url = "http://127.0.0.1:${INDEXER_HTTP_PORT}"

[prover]
sp1_proving_key_path = "/tmp/sp1.key"
groth16_params_path = "/tmp/groth16.params"
mock_prover = true

[observability]
log_level = "info"
metrics_port = 9090
EOF
}

wait_for_http() {
	local url="$1"
	for _ in $(seq 1 90); do
		if curl -fsS "$url" >/dev/null 2>&1; then
			return 0
		fi
		sleep 1
	done
	echo "timed out waiting for HTTP endpoint: $url" >&2
	return 1
}

wait_for_postgres() {
	for _ in $(seq 1 60); do
		if docker exec "${POSTGRES_CONTAINER}" pg_isready -U postgres >/dev/null 2>&1; then
			return 0
		fi
		sleep 1
	done
	echo "timed out waiting for postgres container" >&2
	return 1
}

wait_for_redis() {
	for _ in $(seq 1 60); do
		if docker exec "${REDIS_CONTAINER}" redis-cli ping >/dev/null 2>&1; then
			return 0
		fi
		sleep 1
	done
	echo "timed out waiting for redis container" >&2
	return 1
}

wait_for_validator() {
	for _ in $(seq 1 90); do
		if solana --url "http://127.0.0.1:${RPC_PORT}" slot >/dev/null 2>&1; then
			return 0
		fi
		sleep 1
	done
	echo "timed out waiting for solana-test-validator" >&2
	return 1
}

wait_for_result() {
	local expected="$1"
	export DEMO_RPC_URL="http://127.0.0.1:${RPC_PORT}"
	export DEMO_STATE_JSON="${STATE_JSON}"
	export DEMO_EXPECTED_AVG="${expected}"
	node <<'NODE'
const fs = require('fs');
	const anchor = require('@coral-xyz/anchor');
	const { Connection, PublicKey } = anchor.web3;

function readState(path) {
	return JSON.parse(fs.readFileSync(path, 'utf8'));
}

async function main() {
	const connection = new Connection(process.env.DEMO_RPC_URL, 'confirmed');
	const state = readState(process.env.DEMO_STATE_JSON);
	const resultAccountPubkey = new PublicKey(state.result_account);
	const expected = BigInt(process.env.DEMO_EXPECTED_AVG);

	for (let attempt = 0; attempt < 180; attempt += 1) {
		const accountInfo = await connection.getAccountInfo(resultAccountPubkey, 'confirmed');
		if (accountInfo) {
			const data = Buffer.from(accountInfo.data);
			if (data.length >= 45) {
				let offset = 8;
				offset += 32;
				const resultLen = data.readUInt32LE(offset);
				offset += 4;
				if (data.length >= offset + resultLen + 1) {
					const result = data.subarray(offset, offset + resultLen);
					offset += resultLen;
					const isSet = data.readUInt8(offset) !== 0;
					if (!isSet) {
						await new Promise((resolve) => setTimeout(resolve, 1000));
						continue;
					}
				const value = result.readBigUInt64LE(0);
				console.log(`result_account=${resultAccountPubkey.toBase58()}`);
				console.log(`historical_avg_result=${value.toString()}`);
				console.log(`expected_avg=${expected.toString()}`);
				if (value !== expected) {
					throw new Error(`unexpected result ${value} != ${expected}`);
				}
				process.exit(0);
				}
			}
		}
		await new Promise((resolve) => setTimeout(resolve, 1000));
	}
	throw new Error('timed out waiting for result account');
}

main().catch((error) => {
	console.error(error);
	process.exit(1);
});
NODE
}

build_artifacts() {
	log "building sonar binaries and localnet programs"
	cargo build --bins
	cargo build -p sonar-indexer --lib
	[[ -f "${ROOT_DIR}/target/deploy/sonar_program.so" ]] || {
		echo "missing ${ROOT_DIR}/target/deploy/sonar_program.so; build it before running the demo" >&2
		exit 1
	}
	[[ -f "${ROOT_DIR}/target/deploy/echo_callback.so" ]] || {
		echo "missing ${ROOT_DIR}/target/deploy/echo_callback.so; build it before running the demo" >&2
		exit 1
	}
}

start_containers() {
	cleanup_containers
	log "starting postgres container on port ${POSTGRES_PORT}"
	docker run --rm -d \
		--name "${POSTGRES_CONTAINER}" \
		-e POSTGRES_PASSWORD=postgres \
		-e POSTGRES_DB=postgres \
		-p "127.0.0.1:${POSTGRES_PORT}:5432" \
		postgres:16-alpine >/dev/null

	log "starting redis container on port ${REDIS_PORT}"
	docker run --rm -d \
		--name "${REDIS_CONTAINER}" \
		-p "127.0.0.1:${REDIS_PORT}:6379" \
		redis:7.2.4 >/dev/null

	wait_for_postgres
	wait_for_redis
}

generate_keypairs() {
	solana-keygen new --no-bip39-passphrase --silent --force -o "${COORDINATOR_KEYPAIR}" >/dev/null
	solana-keygen new --no-bip39-passphrase --silent --force -o "${CLIENT_KEYPAIR}" >/dev/null
	solana-keygen new --no-bip39-passphrase --silent --force -o "${OBSERVED_KEYPAIR}" >/dev/null
}

start_validator() {
	cleanup_processes
	if pgrep -f "solana-test-validator.*${LEDGER_DIR}" >/dev/null 2>&1; then
		pkill -f "solana-test-validator.*${LEDGER_DIR}" || true
	fi
	rm -rf "${LEDGER_DIR}"
	mkdir -p "${LEDGER_DIR}"
	log "starting solana-test-validator with geyser plugin"
	solana-test-validator \
		--reset \
		--quiet \
		--ledger "${LEDGER_DIR}" \
		--rpc-port "${RPC_PORT}" \
		--faucet-port "${FAUCET_PORT}" \
		--dynamic-port-range "${DYNAMIC_PORT_START}-${DYNAMIC_PORT_END}" \
		--bind-address 127.0.0.1 \
		--geyser-plugin-config "${PLUGIN_CONFIG_JSON}" \
		--bpf-program "${SONAR_PROGRAM_ID}" "${ROOT_DIR}/target/deploy/sonar_program.so" \
		--bpf-program "${ECHO_CALLBACK_PROGRAM_ID}" "${ROOT_DIR}/target/deploy/echo_callback.so" \
		>"${LOG_DIR}/validator.log" 2>&1 &
	echo $! >"${DEMO_DIR}/validator.pid"
	wait_for_validator
}

fund_keypairs() {
	local coordinator_pubkey client_pubkey
	coordinator_pubkey="$(solana-keygen pubkey "${COORDINATOR_KEYPAIR}")"
	client_pubkey="$(solana-keygen pubkey "${CLIENT_KEYPAIR}")"
	solana airdrop --url "http://127.0.0.1:${RPC_PORT}" 10 "${coordinator_pubkey}" >/dev/null
	solana airdrop --url "http://127.0.0.1:${RPC_PORT}" 10 "${client_pubkey}" >/dev/null
}

start_services() {
	log "starting indexer"
	SONAR_CONFIG="${CONFIG_TOML}" "${ROOT_DIR}/target/debug/sonar-indexer" \
		>"${LOG_DIR}/indexer.log" 2>&1 &
	echo $! >"${DEMO_DIR}/indexer.pid"
	wait_for_http "http://127.0.0.1:${INDEXER_HTTP_PORT}/account_history/11111111111111111111111111111111?from_slot=0&to_slot=0"

	log "starting prover"
	SONAR_CONFIG="${CONFIG_TOML}" SP1_PROVER="${SP1_PROVER}" "${ROOT_DIR}/target/debug/sonar-prover" \
		>"${LOG_DIR}/prover.log" 2>&1 &
	echo $! >"${DEMO_DIR}/prover.pid"

	log "starting coordinator"
	SONAR_CONFIG_PATH="${CONFIG_TOML}" SONAR_COORDINATOR_KEYPAIR_PATH="${COORDINATOR_KEYPAIR}" "${ROOT_DIR}/target/debug/sonar-coordinator" \
		>"${LOG_DIR}/coordinator.log" 2>&1 &
	echo $! >"${DEMO_DIR}/coordinator.pid"

	sleep 3
}

seed_account_history() {
	log "seeding a demo account with slot-specific lamport history"
	export DEMO_RPC_URL="http://127.0.0.1:${RPC_PORT}"
	export DEMO_CLIENT_KEYPAIR="${CLIENT_KEYPAIR}"
	export DEMO_OBSERVED_KEYPAIR="${OBSERVED_KEYPAIR}"
	export DEMO_STATE_JSON="${STATE_JSON}"
	node <<'NODE'
const anchor = require('@coral-xyz/anchor');
const fs = require('fs');
const { Connection, Keypair, LAMPORTS_PER_SOL, SystemProgram, Transaction, sendAndConfirmTransaction } = anchor.web3;

function loadKeypair(path) {
	return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync(path, 'utf8'))));
}

function writeState(path, updates) {
	let state = {};
	if (fs.existsSync(path)) {
		state = JSON.parse(fs.readFileSync(path, 'utf8'));
	}
	Object.assign(state, updates);
	fs.writeFileSync(path, JSON.stringify(state, null, 2));
}

async function waitForNextSlot(connection, currentSlot) {
	for (let i = 0; i < 60; i += 1) {
		const slot = await connection.getSlot('confirmed');
		if (slot > currentSlot) {
			return slot;
		}
		await new Promise((resolve) => setTimeout(resolve, 400));
	}
	throw new Error(`timed out waiting for slot > ${currentSlot}`);
}

async function send(connection, payer, signers, instructions) {
	const blockhash = await connection.getLatestBlockhash('confirmed');
	const tx = new Transaction({ feePayer: payer.publicKey, ...blockhash }).add(...instructions);
	return sendAndConfirmTransaction(connection, tx, signers, { commitment: 'confirmed' });
}

async function main() {
	const connection = new Connection(process.env.DEMO_RPC_URL, 'confirmed');
	const payer = loadKeypair(process.env.DEMO_CLIENT_KEYPAIR);
	const observed = loadKeypair(process.env.DEMO_OBSERVED_KEYPAIR);
	const balances = [];

	await send(connection, payer, [payer, observed], [
		SystemProgram.createAccount({
			fromPubkey: payer.publicKey,
			newAccountPubkey: observed.publicKey,
			lamports: 200_000_000,
			space: 0,
			programId: SystemProgram.programId,
		})
	]);
	let slot = await connection.getSlot('confirmed');
	balances.push({ slot, lamports: await connection.getBalance(observed.publicKey, 'confirmed') });

	await waitForNextSlot(connection, slot);
	await send(connection, payer, [payer], [
		SystemProgram.transfer({ fromPubkey: payer.publicKey, toPubkey: observed.publicKey, lamports: 80_000_000 })
	]);
	slot = await connection.getSlot('confirmed');
	balances.push({ slot, lamports: await connection.getBalance(observed.publicKey, 'confirmed') });

	await waitForNextSlot(connection, slot);
	await send(connection, payer, [payer, observed], [
		SystemProgram.transfer({ fromPubkey: observed.publicKey, toPubkey: payer.publicKey, lamports: 50_000_000 })
	]);
	slot = await connection.getSlot('confirmed');
	balances.push({ slot, lamports: await connection.getBalance(observed.publicKey, 'confirmed') });

	await waitForNextSlot(connection, slot);
	await send(connection, payer, [payer], [
		SystemProgram.transfer({ fromPubkey: payer.publicKey, toPubkey: observed.publicKey, lamports: 170_000_000 })
	]);
	slot = await connection.getSlot('confirmed');
	balances.push({ slot, lamports: await connection.getBalance(observed.publicKey, 'confirmed') });

	const values = balances.map((item) => item.lamports);
	const expectedAvg = values.reduce((acc, value) => acc + value, 0) / values.length;
	const toSlot = balances[balances.length - 1].slot;
	writeState(process.env.DEMO_STATE_JSON, {
		observed_pubkey: observed.publicKey.toBase58(),
		from_slot: 0,
		to_slot: toSlot,
		expected_avg: String(expectedAvg),
		seeded_balances: JSON.stringify(values),
	});
	console.log(`observed_pubkey=${observed.publicKey.toBase58()}`);
	console.log(`from_slot=0`);
	console.log(`to_slot=${toSlot}`);
	console.log(`expected_avg=${expectedAvg}`);
}

main().catch((error) => {
	console.error(error);
	process.exit(1);
});
NODE
}

request_historical_avg() {
	log "sending historical-average request directly to the sonar program"
	export DEMO_RPC_URL="http://127.0.0.1:${RPC_PORT}"
	export DEMO_CLIENT_KEYPAIR="${CLIENT_KEYPAIR}"
	export DEMO_STATE_JSON="${STATE_JSON}"
	export DEMO_SONAR_PROGRAM_ID="${SONAR_PROGRAM_ID}"
	export DEMO_CALLBACK_PROGRAM_ID="${ECHO_CALLBACK_PROGRAM_ID}"
	node <<'NODE'
const anchor = require('@coral-xyz/anchor');
const fs = require('fs');
const crypto = require('crypto');
const { Connection, Keypair, PublicKey, SystemProgram, Transaction, TransactionInstruction, sendAndConfirmTransaction } = anchor.web3;

function loadKeypair(path) {
	return Keypair.fromSecretKey(Uint8Array.from(JSON.parse(fs.readFileSync(path, 'utf8'))));
}

function readState(path) {
	return JSON.parse(fs.readFileSync(path, 'utf8'));
}

function writeState(path, updates) {
	const state = readState(path);
	Object.assign(state, updates);
	fs.writeFileSync(path, JSON.stringify(state, null, 2));
}

function discriminator(name) {
	return crypto.createHash('sha256').update(`global:${name}`).digest().subarray(0, 8);
}

function encodeU64(value) {
	const buffer = Buffer.alloc(8);
	buffer.writeBigUInt64LE(BigInt(value));
	return buffer;
}

function encodeBytes(bytes) {
	const length = Buffer.alloc(4);
	length.writeUInt32LE(bytes.length, 0);
	return Buffer.concat([length, Buffer.from(bytes)]);
}

async function send(connection, payer, instructions) {
	const blockhash = await connection.getLatestBlockhash('confirmed');
	const tx = new Transaction({ feePayer: payer.publicKey, ...blockhash }).add(...instructions);
	return sendAndConfirmTransaction(connection, tx, [payer], { commitment: 'confirmed' });
}

async function main() {
	const connection = new Connection(process.env.DEMO_RPC_URL, 'confirmed');
	const payer = loadKeypair(process.env.DEMO_CLIENT_KEYPAIR);
	const state = readState(process.env.DEMO_STATE_JSON);
	const requestId = crypto.randomBytes(32);
	const sonarProgramId = new PublicKey(process.env.DEMO_SONAR_PROGRAM_ID);
	const callbackProgramId = new PublicKey(process.env.DEMO_CALLBACK_PROGRAM_ID);
	const observed = new PublicKey(state.observed_pubkey);

	const [requestMetadata] = PublicKey.findProgramAddressSync([Buffer.from('request'), requestId], sonarProgramId);
	const [resultAccount] = PublicKey.findProgramAddressSync([Buffer.from('result'), requestId], sonarProgramId);
	const computationId = Buffer.from([180, 134, 237, 198, 23, 219, 85, 143, 84, 245, 61, 62, 222, 122, 82, 179, 3, 201, 204, 111, 144, 62, 32, 159, 91, 227, 160, 78, 252, 195, 98, 100]);
	const rawInputs = Buffer.concat([
		observed.toBuffer(),
		encodeU64(state.from_slot),
		encodeU64(state.to_slot),
	]);

	const payload = Buffer.concat([
		discriminator('request'),
		requestId,
		computationId,
		encodeBytes(rawInputs),
		encodeU64((await connection.getSlot('confirmed')) + 500),
		encodeU64(2_000_000),
	]);

	const instruction = new TransactionInstruction({
		programId: sonarProgramId,
		keys: [
			{ pubkey: payer.publicKey, isSigner: true, isWritable: true },
			{ pubkey: callbackProgramId, isSigner: false, isWritable: false },
			{ pubkey: requestMetadata, isSigner: false, isWritable: true },
			{ pubkey: resultAccount, isSigner: false, isWritable: true },
			{ pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
		],
		data: payload,
	});

	await send(connection, payer, [instruction]);

	writeState(process.env.DEMO_STATE_JSON, {
		request_id_hex: requestId.toString('hex'),
		request_metadata: requestMetadata.toBase58(),
		result_account: resultAccount.toBase58(),
	});

	console.log(`request_id_hex=${requestId.toString('hex')}`);
	console.log(`request_metadata=${requestMetadata.toBase58()}`);
	console.log(`result_account=${resultAccount.toBase58()}`);
}

main().catch((error) => {
	console.error(error);
	process.exit(1);
});
NODE
}

print_status() {
	cat <<EOF
Demo directory: ${DEMO_DIR}
Validator RPC: http://127.0.0.1:${RPC_PORT}
Indexer URL: http://127.0.0.1:${INDEXER_HTTP_PORT}
Observed account: $(read_state observed_pubkey 2>/dev/null || true)
Request metadata PDA: $(read_state request_metadata 2>/dev/null || true)
Result PDA: $(read_state result_account 2>/dev/null || true)
Expected average: $(read_state expected_avg 2>/dev/null || true)
EOF
}

tail_logs() {
	exec tail -f "${LOG_DIR}/validator.log" "${LOG_DIR}/indexer.log" "${LOG_DIR}/prover.log" "${LOG_DIR}/coordinator.log"
}

start_stack() {
	need_cmd docker
	need_cmd solana
	need_cmd solana-test-validator
	need_cmd solana-keygen
	need_cmd curl
	need_cmd python3
	need_cmd node

	cleanup_all
	rm -rf "${DEMO_DIR}"
	mkdir -p "${LOG_DIR}" "${LEDGER_DIR}"

	build_artifacts
	start_containers
	write_plugin_config
	write_runtime_config
	generate_keypairs
	start_validator
	fund_keypairs
	start_services
	seed_account_history
	print_status
}

demo() {
	start_stack
	request_historical_avg
	local expected
	expected="$(read_state expected_avg)"
	wait_for_result "${expected}"
	echo
	log "proof generation appears in ${LOG_DIR}/prover.log"
	log "callback submission / verification appears in ${LOG_DIR}/coordinator.log and ${LOG_DIR}/validator.log"
	echo
	read -r -p "Press enter to stop the demo stack..." _
}

command_name="${1:-demo}"

case "${command_name}" in
	start)
		start_stack
		KEEP_ALIVE_ON_EXIT=1
		;;
	request)
		KEEP_ALIVE_ON_EXIT=1
		request_historical_avg
		;;
	result)
		KEEP_ALIVE_ON_EXIT=1
		wait_for_result "$(read_state expected_avg)"
		;;
	status)
		KEEP_ALIVE_ON_EXIT=1
		print_status
		;;
	logs)
		KEEP_ALIVE_ON_EXIT=1
		tail_logs
		;;
	stop)
		cleanup_all
		KEEP_ALIVE_ON_EXIT=1
		;;
	demo)
		demo
		;;
	*)
		cat <<EOF
Usage: $0 [start|request|result|status|logs|stop|demo]

	start   Build, start validator + services, deploy programs, and seed demo balances
	request Submit the historical-average request through the client program
	result  Wait for the callback and print the decoded average
	status  Print the current demo PDAs and expected average
	logs    Tail validator and service logs
	stop    Stop the validator, services, and docker containers
	demo    Run the full flow, then wait for enter before shutting down
EOF
		exit 1
		;;
esac
