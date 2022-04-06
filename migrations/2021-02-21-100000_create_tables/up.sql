CREATE TABLE IF NOT EXISTS block (
    id                      SERIAL      UNIQUE,
    hash                    BYTEA       PRIMARY KEY,
    prev_hash               BYTEA       NOT NULL,
    height                  INTEGER     NOT NULL,
    tags                    INTEGER[]   NOT NULL,
    missing_tx              INTEGER     NOT NULL,
    extra_tx                INTEGER     NOT NULL,
    shared_tx               INTEGER     NOT NULL,
    sanctioned_missing_tx   INTEGER     NOT NULL,
    equality                REAL        NOT NULL,
    block_time              TIMESTAMP   NOT NULL,
    block_seen_time         TIMESTAMP   NOT NULL,
    block_tx                INTEGER     NOT NULL,
    block_sanctioned        INTEGER     NOT NULL,
    block_cb_value          BIGINT      NOT NULL,
    block_cb_fees           BIGINT      NOT NULL,
    block_weight            INTEGER     NOT NULL,
    block_pkg_weights       BIGINT[]   NOT NULL,
    block_pkg_feerates      REAL[]      NOT NULL,
    pool_name               TEXT        NOT NULL,
    pool_link               TEXT        NOT NULL,
    pool_id_method          TEXT        NOT NULL,
    template_tx             INTEGER     NOT NULL,
    template_time           TIMESTAMP   NOT NULL,
    template_sanctioned     INTEGER     NOT NULL,
    template_cb_value       BIGINT      NOT NULL,
    template_cb_fees        BIGINT      NOT NULL,
    template_weight         INTEGER     NOT NULL,
    template_pkg_weights    BIGINT[]   NOT NULL,
    template_pkg_feerates   REAL[]      NOT NULL
);

CREATE TABLE IF NOT EXISTS transaction (
    txid            BYTEA       PRIMARY KEY,
    sanctioned      BOOL        NOT NULL,
    vsize           INTEGER     NOT NULL,
    fee             BIGINT      NOT NULL,
    output_sum      BIGINT      NOT NULL,
    tags            INTEGER[]   NOT NULL,
    input_count     INTEGER     NOT NULL,
    inputs          TEXT[]      NOT NULL,
    output_count    INTEGER     NOT NULL,
    outputs         TEXT[]      NOT NULL
);

CREATE TABLE IF NOT EXISTS transaction_only_in_block (
    block_id            INTEGER REFERENCES block(id),
    position            INTEGER NOT NULL,
    transaction_txid    BYTEA   REFERENCES transaction(txid),
    PRIMARY KEY (block_id, transaction_txid)
);

CREATE TABLE IF NOT EXISTS transaction_only_in_template (
    block_id            INTEGER     REFERENCES block(id),
    position            INTEGER     NOT NULL,
    mempool_age_seconds INTEGER     NOT NULL,
    transaction_txid    BYTEA       REFERENCES transaction(txid),
    PRIMARY KEY (block_id, transaction_txid)
);

CREATE TABLE IF NOT EXISTS sanctioned_transaction_info (
    block_id            INTEGER REFERENCES block(id),
    transaction_txid    BYTEA   REFERENCES transaction(txid),
    in_block            BOOL    NOT NULL,
    in_template         BOOL    NOT NULL,
    addresses           TEXT[]  NOT NULL,
    PRIMARY KEY (block_id, transaction_txid)
);

CREATE TABLE IF NOT EXISTS conflicting_transactions (
    block_id                        INTEGER REFERENCES block(id),
    template_txids                  BYTEA[] NOT NULL,
    block_txids                     BYTEA[] NOT NULL,
    conflicting_outpoints_txids     BYTEA[] NOT NULL,
    conflicting_outpoints_vouts     INTEGER[] NOT NULL,
    PRIMARY KEY (block_id, template_txids, block_txids)
);

CREATE TABLE IF NOT EXISTS sanctioned_utxo_scan_info (
    end_time                        TIMESTAMP NOT NULL,
    end_height                      INTEGER NOT NULL,
    duration_seconds                INTEGER NOT NULL,
    utxo_amount                     BIGINT NOT NULL,
    utxo_count                      INTEGER NOT NULL,
    PRIMARY KEY (end_time)
);

CREATE TABLE IF NOT EXISTS sanctioned_utxo (
    txid                            BYTEA NOT NULL,
    vout                            INTEGER NOT NULL,
    script_pubkey                   BYTEA NOT NULL,
    amount                          BIGINT NOT NULL,
    height                          INTEGER NOT NULL,
    PRIMARY KEY (txid, vout)
);

CREATE TABLE IF NOT EXISTS debug_template_selection (
    block_id            INTEGER     REFERENCES block(id),
    template_time       TIMESTAMP   NOT NULL,
    count_missing       INTEGER     NOT NULL,
    count_shared        INTEGER     NOT NULL,
    count_extra         INTEGER     NOT NULL,
    selected            BOOLEAN     NOT NULL,
    PRIMARY KEY (block_id, template_time)
);

CREATE INDEX IF NOT EXISTS block_height_desc_index ON block(height DESC);
