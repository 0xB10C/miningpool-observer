//! Fixtures and helpers shared between the tests of this crate.

use miningpool_observer_shared::chrono::NaiveDateTime;
use miningpool_observer_shared::model::Transaction;

use crate::model::MissingTransactionBlockInfo;

/// A fixed point in time used by the fixtures: 2021-11-14 09:12:11 UTC.
pub fn time() -> NaiveDateTime {
    NaiveDateTime::parse_from_str("2021-11-14 09:12:11", "%Y-%m-%d %H:%M:%S").unwrap()
}

/// A 32 byte hash where every byte is `byte`.
pub fn hash(byte: u8) -> Vec<u8> {
    vec![byte; 32]
}

/// A transaction with all fields filled in. The `tags` are passed in as the
/// templates render a tag for each of them.
pub fn transaction_fixture(tags: Vec<i32>) -> Transaction {
    Transaction {
        txid: hash(0xaa),
        sanctioned: false,
        vsize: 300,
        fee: 1000,
        output_sum: 500_000_000,
        tags,
        input_count: 2,
        inputs: vec!["P2WPKH 0.1 BTC".to_string(), "P2TR 0.2 BTC".to_string()],
        output_count: 1,
        outputs: vec!["P2WPKH 0.3 BTC".to_string()],
        sigops: 4,
    }
}

/// Information about a block a transaction is missing from.
pub fn missing_transaction_block_info_fixture(height: i32) -> MissingTransactionBlockInfo {
    MissingTransactionBlockInfo {
        hash: hash(height as u8),
        time: time(),
        height,
        pool: "Test Pool".to_string(),
        template_position: 7,
        mempool_age: 120,
        template_tx_count: 2000,
        last_block_pkg_feerate: 4.2,
    }
}
