CREATE TABLE IF NOT EXISTS account_history (
    slot BIGINT NOT NULL,
    pubkey BYTEA NOT NULL,
    lamports BIGINT NOT NULL,
    owner BYTEA NOT NULL,
    executable BOOLEAN NOT NULL,
    rent_epoch BIGINT NOT NULL,
    data_hash BYTEA NOT NULL,
    write_version BIGINT NOT NULL,
    PRIMARY KEY (slot, pubkey, write_version),
    CONSTRAINT account_history_pubkey_len CHECK (octet_length(pubkey) = 32),
    CONSTRAINT account_history_owner_len CHECK (octet_length(owner) = 32),
    CONSTRAINT account_history_data_hash_len CHECK (octet_length(data_hash) = 32)
);

CREATE INDEX IF NOT EXISTS account_history_pubkey_slot_idx
    ON account_history (pubkey, slot DESC, write_version DESC);

CREATE INDEX IF NOT EXISTS account_history_slot_idx
    ON account_history (slot);

CREATE TABLE IF NOT EXISTS slot_metadata (
    slot BIGINT PRIMARY KEY,
    blockhash BYTEA,
    parent_slot BIGINT,
    timestamp TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'processed'
);

CREATE TABLE IF NOT EXISTS request_tracking (
    request_id BYTEA PRIMARY KEY,
    slot_requested BIGINT NOT NULL,
    status TEXT NOT NULL,
    proof_tx_sig TEXT
);
