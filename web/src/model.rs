use miningpool_observer_shared::bitcoincore_rpc::json::serde_hex;
use miningpool_observer_shared::chrono::NaiveDateTime;
use miningpool_observer_shared::diesel;
use miningpool_observer_shared::diesel::sql_types::{BigInt, Double, Text};
use miningpool_observer_shared::diesel::{deserialize::Queryable, deserialize::QueryableByName};
use miningpool_observer_shared::model;
use serde::Serialize;

// Web-only models

#[derive(Debug, Clone, Serialize)]
pub struct MissingTransactionBlockInfo {
    #[serde(with = "serde_hex")]
    pub hash: Vec<u8>,
    pub time: NaiveDateTime,
    pub height: i32,
    pub pool: String,
    pub template_position: i32,
    pub mempool_age: i32,
    pub template_tx_count: i32,
    pub last_block_pkg_feerate: f32,
}

#[derive(Debug, Serialize)]
pub struct MissingTransaction {
    pub transaction: model::Transaction,
    pub blocks: Vec<MissingTransactionBlockInfo>,
}

impl MissingTransaction {
    /// Height of the newest block the transaction is missing from. This is
    /// only used as a sort key. Returns 0 if the transaction isn't missing
    /// from any block.
    pub fn max_height(&self) -> i32 {
        self.blocks.iter().map(|b| b.height).max().unwrap_or(0)
    }
}

#[derive(Debug, QueryableByName, Queryable, Serialize)]
pub struct AvgPoolFees {
    #[diesel(sql_type = Text)]
    pub pool_name: String,
    #[diesel(sql_type = BigInt)]
    pub count: i64,
    #[diesel(sql_type = Double)]
    pub median: f64,
    #[diesel(sql_type = Double)]
    pub q1: f64,
    #[diesel(sql_type = Double)]
    pub q3: f64,
}

#[derive(Serialize)]
pub struct MissingSanctionedTransaction {
    pub transaction: model::Transaction,
    pub missing_info: model::TransactionOnlyInTemplate,
    pub addresses: Vec<String>,
}

#[derive(Debug, QueryableByName, Queryable, Serialize)]
pub struct PoolSanctionedTableEntry {
    #[diesel(sql_type = Text)]
    pub pool_name: String,
    #[diesel(sql_type = BigInt)]
    pub in_both: i64,
    #[diesel(sql_type = BigInt)]
    pub only_in_block: i64,
    #[diesel(sql_type = BigInt)]
    pub only_in_template: i64,
}

#[derive(Serialize)]
pub struct ConflictingOutpoint {
    #[serde(with = "serde_hex")]
    pub txid: Vec<u8>,
    pub vout: i32,
}

#[derive(Serialize)]
pub struct ConflictingTransactionSet {
    pub template_transactions: Vec<model::Transaction>,
    pub block_transactions: Vec<model::Transaction>,
    pub conflicting_outpoints: Vec<ConflictingOutpoint>,
}

#[derive(Serialize)]
pub struct ConflictingTranscationInfo {
    pub block: model::Block,
    pub conflicting_transaction_sets: Vec<ConflictingTransactionSet>,
}

#[derive(Serialize)]
pub struct DebugTemplateSelectionInfosAndBlock {
    pub block: model::Block,
    pub infos: Vec<model::DebugTemplateSelectionInfo>,
}

#[derive(Serialize)]
pub struct BlockWithTx {
    pub block: model::Block,
    pub txns_only_in_template: Vec<(model::TransactionOnlyInTemplate, model::Transaction)>,
    pub txns_only_in_block: Vec<(model::TransactionOnlyInBlock, model::Transaction)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{missing_transaction_block_info_fixture, transaction_fixture};

    fn missing_transaction(heights: &[i32]) -> MissingTransaction {
        MissingTransaction {
            transaction: transaction_fixture(vec![]),
            blocks: heights
                .iter()
                .map(|h| missing_transaction_block_info_fixture(*h))
                .collect(),
        }
    }

    #[test]
    fn test_missing_transaction_max_height() {
        assert_eq!(missing_transaction(&[700_000]).max_height(), 700_000);
        assert_eq!(
            missing_transaction(&[700_000, 700_002, 700_001]).max_height(),
            700_002
        );
        // the heights don't have to be sorted
        assert_eq!(
            missing_transaction(&[700_002, 700_000]).max_height(),
            700_002
        );
    }

    #[test]
    fn test_missing_transaction_max_height_without_blocks() {
        // A transaction that's missing from no block at all shouldn't panic.
        // The max height is only used as a sort key.
        assert_eq!(missing_transaction(&[]).max_height(), 0);
    }
}
