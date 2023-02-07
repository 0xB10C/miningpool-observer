use super::schema::{
    block, conflicting_transactions, debug_template_selection, sanctioned_transaction_info,
    sanctioned_utxo, sanctioned_utxo_scan_info, transaction, transaction_only_in_block,
    transaction_only_in_template,
};

use bitcoincore_rpc::json::serde_hex;
use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::Serialize;

use std::hash::{Hash, Hasher};

// Database models shared between web and daemon.

/// This is used to query a block from the database. Use [NewBlock] for inserting.
#[derive(Queryable, Serialize, Identifiable)]
#[diesel(primary_key(hash))]
#[diesel(table_name = block)]
pub struct Block {
    pub id: i32,
    #[serde(with = "serde_hex")]
    pub hash: Vec<u8>,
    #[serde(with = "serde_hex")]
    pub prev_hash: Vec<u8>,
    pub height: i32,
    pub tags: Vec<i32>,
    pub missing_tx: i32,
    pub extra_tx: i32,
    pub shared_tx: i32,
    /// Count of sanctioned transactions present in the template,
    /// but not present in the block.
    pub sanctioned_missing_tx: i32,
    pub equality: f32,
    pub block_time: NaiveDateTime,
    pub block_seen_time: NaiveDateTime,
    /// Count of transactions in the block.
    pub block_tx: i32,
    /// Count of sanctioned transactions included in the block.
    pub block_sanctioned: i32,
    /// Coinbase output value.
    pub block_cb_value: i64,
    pub block_cb_fees: i64,
    pub block_weight: i32,
    pub block_pkg_weights: Vec<i64>,
    pub block_pkg_feerates: Vec<f32>,
    pub pool_name: String,
    pub pool_link: String,
    pub pool_id_method: String,
    /// Count of transactions in the template.
    pub template_tx: i32,
    pub template_time: NaiveDateTime,
    pub template_sanctioned: i32,
    /// Coinbase output value.
    pub template_cb_value: i64,
    pub template_cb_fees: i64,
    pub template_weight: i32,
    pub template_pkg_weights: Vec<i64>,
    pub template_pkg_feerates: Vec<f32>,
}

/// This is used to construct a [Block] for insertion into the database.
/// Compared to [Block] this does not contain the id field.
/// diesel.rs needs two types for inserting something with and SERIAL as id.
#[derive(Insertable, Serialize, Debug)]
#[diesel(table_name = block)]
pub struct NewBlock {
    #[serde(with = "serde_hex")]
    pub hash: Vec<u8>,
    #[serde(with = "serde_hex")]
    pub prev_hash: Vec<u8>,
    pub height: i32,
    pub tags: Vec<i32>,
    pub missing_tx: i32,
    pub extra_tx: i32,
    pub shared_tx: i32,
    /// Count of sanctioned transactions present in the template,
    /// but not present in the block.
    pub sanctioned_missing_tx: i32,
    pub equality: f32,
    pub block_time: NaiveDateTime,
    pub block_seen_time: NaiveDateTime,
    /// Count of transactions in the block.
    pub block_tx: i32,
    /// Count of sanctioned transactions included in the block.
    pub block_sanctioned: i32,
    /// Coinbase output value.
    pub block_cb_value: i64,
    pub block_cb_fees: i64,
    pub block_weight: i32,
    pub block_pkg_weights: Vec<i64>,
    pub block_pkg_feerates: Vec<f32>,
    pub pool_name: String,
    pub pool_link: String,
    pub pool_id_method: String,
    /// Count of transactions in the template.
    pub template_tx: i32,
    pub template_time: NaiveDateTime,
    pub template_sanctioned: i32,
    /// Coinbase output value.
    pub template_cb_value: i64,
    pub template_cb_fees: i64,
    pub template_weight: i32,
    pub template_pkg_weights: Vec<i64>,
    pub template_pkg_feerates: Vec<f32>,
}

#[derive(Debug, Insertable, Queryable, Serialize, Clone)]
#[diesel(table_name = transaction)]
pub struct Transaction {
    #[serde(with = "serde_hex")]
    pub txid: Vec<u8>,
    pub sanctioned: bool,
    pub vsize: i32,
    pub fee: i64,
    pub output_sum: i64,
    pub tags: Vec<i32>,
    pub input_count: i32,
    pub inputs: Vec<String>,
    pub output_count: i32,
    pub outputs: Vec<String>,
}

impl Hash for Transaction {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.txid.hash(state);
    }
}

#[derive(Insertable, Queryable, Serialize)]
#[diesel(table_name = transaction_only_in_block)]
pub struct TransactionOnlyInBlock {
    pub block_id: i32,
    pub position: i32,
    pub transaction_txid: Vec<u8>,
}

#[derive(Insertable, Queryable, Serialize, Clone)]
#[diesel(table_name = transaction_only_in_template)]
pub struct TransactionOnlyInTemplate {
    pub block_id: i32,
    pub position: i32,
    pub mempool_age_seconds: i32,
    pub transaction_txid: Vec<u8>,
}

#[derive(Insertable, Queryable, Serialize)]
#[diesel(table_name = sanctioned_transaction_info)]
pub struct SanctionedTransactionInfo {
    pub block_id: i32,
    pub transaction_txid: Vec<u8>,
    pub in_block: bool,
    pub in_template: bool,
    pub addresses: Vec<String>,
}

#[derive(Insertable, Queryable, Serialize, Debug)]
#[diesel(table_name = conflicting_transactions)]
pub struct ConflictingTransaction {
    pub block_id: i32,
    pub template_txids: Vec<Vec<u8>>,
    pub block_txids: Vec<Vec<u8>>,
    pub conflicting_outpoints_txids: Vec<Vec<u8>>,
    pub conflicting_outpoints_vouts: Vec<i32>,
}

#[derive(Insertable, Queryable, Serialize, Debug)]
#[diesel(table_name = sanctioned_utxo)]
pub struct SanctionedUtxo {
    #[serde(with = "serde_hex")]
    pub txid: Vec<u8>,
    pub vout: i32,
    pub script_pubkey: Vec<u8>,
    pub amount: i64,
    pub height: i32,
}

#[derive(Insertable, Queryable, Serialize, Debug)]
#[diesel(table_name = sanctioned_utxo_scan_info)]
pub struct SanctionedUtxoScanInfo {
    pub end_time: NaiveDateTime,
    pub end_height: i32,
    pub duration_seconds: i32,
    pub utxo_amount: i64,
    pub utxo_count: i32,
}

#[derive(Insertable, Queryable, Serialize, Debug, Clone)]
#[diesel(table_name = debug_template_selection)]
pub struct DebugTemplateSelectionInfo {
    pub block_id: i32,
    pub template_time: NaiveDateTime,
    pub count_missing: i32,
    pub count_shared: i32,
    pub count_extra: i32,
    pub selected: bool,
}
