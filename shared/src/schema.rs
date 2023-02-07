use diesel::prelude::*;

table! {
    block (hash) {
        id -> Int4,
        hash -> Bytea,
        prev_hash -> Bytea,
        height -> Int4,
        tags -> Array<Int4>,
        missing_tx -> Int4,
        extra_tx -> Int4,
        shared_tx -> Int4,
        sanctioned_missing_tx -> Int4,
        equality -> Float4,
        block_time -> Timestamp,
        block_seen_time -> Timestamp,
        block_tx -> Int4,
        block_sanctioned -> Int4,
        block_cb_value -> Int8,
        block_cb_fees -> Int8,
        block_weight -> Int4,
        block_pkg_weights -> Array<Int8>,
        block_pkg_feerates -> Array<Float4>,
        pool_name -> Text,
        pool_link -> Text,
        pool_id_method -> Text,
        template_tx -> Int4,
        template_time -> Timestamp,
        template_sanctioned -> Int4,
        template_cb_value -> Int8,
        template_cb_fees -> Int8,
        template_weight -> Int4,
        template_pkg_weights -> Array<Int8>,
        template_pkg_feerates -> Array<Float4>,
    }
}

table! {
    conflicting_transactions (block_id, template_txids, block_txids) {
        block_id -> Int4,
        template_txids -> Array<Bytea>,
        block_txids -> Array<Bytea>,
        conflicting_outpoints_txids -> Array<Bytea>,
        conflicting_outpoints_vouts -> Array<Int4>,
    }
}

table! {
    debug_template_selection (block_id, template_time) {
        block_id -> Int4,
        template_time -> Timestamp,
        count_missing -> Int4,
        count_shared -> Int4,
        count_extra -> Int4,
        selected -> Bool,
    }
}

table! {
    node_info (id) {
        id -> Int4,
        version -> Text,
    }
}

table! {
    sanctioned_transaction_info (block_id, transaction_txid) {
        block_id -> Int4,
        transaction_txid -> Bytea,
        in_block -> Bool,
        in_template -> Bool,
        addresses -> Array<Text>,
    }
}

table! {
    sanctioned_utxo (txid, vout) {
        txid -> Bytea,
        vout -> Int4,
        script_pubkey -> Bytea,
        amount -> Int8,
        height -> Int4,
    }
}

table! {
    sanctioned_utxo_scan_info (end_time) {
        end_time -> Timestamp,
        end_height -> Int4,
        duration_seconds -> Int4,
        utxo_amount -> Int8,
        utxo_count -> Int4,
    }
}

table! {
    transaction (txid) {
        txid -> Bytea,
        sanctioned -> Bool,
        vsize -> Int4,
        fee -> Int8,
        output_sum -> Int8,
        tags -> Array<Int4>,
        input_count -> Int4,
        inputs -> Array<Text>,
        output_count -> Int4,
        outputs -> Array<Text>,
    }
}

table! {
    transaction_only_in_block (block_id, transaction_txid) {
        block_id -> Int4,
        position -> Int4,
        transaction_txid -> Bytea,
    }
}

table! {
    transaction_only_in_template (block_id, transaction_txid) {
        block_id -> Int4,
        position -> Int4,
        mempool_age_seconds -> Int4,
        transaction_txid -> Bytea,
    }
}

joinable!(sanctioned_transaction_info -> transaction (transaction_txid));
joinable!(transaction_only_in_block -> transaction (transaction_txid));
joinable!(transaction_only_in_template -> transaction (transaction_txid));

allow_tables_to_appear_in_same_query!(
    block,
    conflicting_transactions,
    debug_template_selection,
    node_info,
    sanctioned_transaction_info,
    sanctioned_utxo,
    sanctioned_utxo_scan_info,
    transaction,
    transaction_only_in_block,
    transaction_only_in_template,
);
