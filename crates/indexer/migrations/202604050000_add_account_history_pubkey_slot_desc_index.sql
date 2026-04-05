DROP INDEX IF EXISTS account_history_pubkey_slot_idx;

CREATE INDEX IF NOT EXISTS account_history_pubkey_slot_desc_idx
    ON account_history (pubkey, slot DESC);