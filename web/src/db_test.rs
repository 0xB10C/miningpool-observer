//! Integration tests for the database queries.
//!
//! They need a PostgreSQL database and are skipped when the environment
//! variable TEST_DATABASE_URL isn't set. `contrib/test-db.sh` starts a
//! throwaway database:
//!
//! ```text
//! eval "$(contrib/test-db.sh start)"
//! cargo test --all-features
//! contrib/test-db.sh stop
//! ```

use miningpool_observer_shared::chrono::{Duration, NaiveDateTime};
use miningpool_observer_shared::diesel;
use miningpool_observer_shared::diesel::pg::PgConnection;
use miningpool_observer_shared::diesel::prelude::*;
use miningpool_observer_shared::model::{
    ConflictingTransaction, NewBlock, SanctionedAddress, SanctionedTransactionInfo,
    SanctionedUtxoScanInfo, Transaction, TransactionOnlyInBlock, TransactionOnlyInTemplate,
};
use miningpool_observer_shared::schema;

use crate::db;
use crate::test_util::*;

/// Returns a connection to the test database or leaves the test early when
/// there is no test database.
macro_rules! conn_or_skip {
    () => {
        match test_conn() {
            Some(conn) => conn,
            None => return,
        }
    };
}

/// Asserts that a query didn't find anything. Not every type the queries
/// return implements Debug, so this can't be an assert_eq!.
fn assert_not_found<T>(result: Result<T, diesel::result::Error>) {
    match result {
        Err(diesel::result::Error::NotFound) => (),
        Err(e) => panic!("expected a NotFound error, got {:?}", e),
        Ok(_) => panic!("expected a NotFound error, but something was found"),
    }
}

//##### FIXTURE HELPERS

/// A 32 byte hash that's distinct for every `n`.
fn hash_n(n: u8) -> Vec<u8> {
    let mut h = vec![0u8; 32];
    h[0] = n;
    h[1] = 0xab;
    h
}

fn time_plus(minutes: i64) -> NaiveDateTime {
    time() + Duration::minutes(minutes)
}

/// A block that's distinct for every `n`.
fn new_block(n: u8, height: i32, pool: &str) -> NewBlock {
    NewBlock {
        hash: hash_n(n),
        prev_hash: hash_n(n.wrapping_sub(1)),
        height,
        block_time: time_plus(height as i64),
        block_seen_time: time_plus(height as i64),
        pool_name: pool.to_string(),
        ..new_block_fixture()
    }
}

fn insert_block(conn: &mut PgConnection, block: &NewBlock) -> i64 {
    diesel::insert_into(schema::block::table)
        .values(block)
        .returning(schema::block::id)
        .get_result(conn)
        .expect("should be able to insert a block")
}

fn insert_transaction(conn: &mut PgConnection, tx: &Transaction) {
    diesel::insert_into(schema::transaction::table)
        .values(tx)
        .execute(conn)
        .expect("should be able to insert a transaction");
}

/// Inserts a transaction with the txid `hash_n(n)`.
fn insert_transaction_n(conn: &mut PgConnection, n: u8) -> Vec<u8> {
    let txid = hash_n(n);
    let mut tx = transaction_fixture(vec![]);
    tx.txid = txid.clone();
    insert_transaction(conn, &tx);
    txid
}

fn insert_only_in_template(conn: &mut PgConnection, block_id: i64, txid: &[u8], position: i32) {
    diesel::insert_into(schema::transaction_only_in_template::table)
        .values(TransactionOnlyInTemplate {
            block_id,
            position,
            mempool_age_seconds: 120,
            transaction_txid: txid.to_vec(),
        })
        .execute(conn)
        .expect("should be able to insert a transaction_only_in_template");
}

fn insert_only_in_block(conn: &mut PgConnection, block_id: i64, txid: &[u8], position: i32) {
    diesel::insert_into(schema::transaction_only_in_block::table)
        .values(TransactionOnlyInBlock {
            block_id,
            position,
            transaction_txid: txid.to_vec(),
        })
        .execute(conn)
        .expect("should be able to insert a transaction_only_in_block");
}

//##### BLOCKS

#[test]
fn test_blocks_pagination() {
    let mut conn = conn_or_skip!();

    // one more block than fits on a page
    let count = db::MAX_BLOCKS_PER_PAGE as u8 + 1;
    for n in 0..count {
        insert_block(&mut conn, &new_block(n, 700_000 + n as i32, "Test Pool"));
    }

    let (first_page, max_pages) = db::blocks(&mut conn, 0).unwrap();
    assert_eq!(first_page.len(), db::MAX_BLOCKS_PER_PAGE as usize);
    assert_eq!(max_pages, 2);
    // newest block first
    assert_eq!(first_page[0].height, 700_000 + count as i32 - 1);
    let heights: Vec<i32> = first_page.iter().map(|b| b.height).collect();
    let mut sorted = heights.clone();
    sorted.sort_unstable();
    sorted.reverse();
    assert_eq!(heights, sorted);

    let (second_page, max_pages) = db::blocks(&mut conn, 1).unwrap();
    assert_eq!(second_page.len(), 1);
    assert_eq!(second_page[0].height, 700_000);
    assert_eq!(max_pages, 2);

    let (third_page, _) = db::blocks(&mut conn, 2).unwrap();
    assert!(third_page.is_empty());
}

#[test]
fn test_blocks_without_blocks() {
    let mut conn = conn_or_skip!();

    let (blocks, max_pages) = db::blocks(&mut conn, 0).unwrap();
    assert!(blocks.is_empty());
    assert_eq!(max_pages, 0);
}

#[test]
fn test_blocks_by_pool() {
    let mut conn = conn_or_skip!();

    insert_block(&mut conn, &new_block(1, 700_001, "Pool A"));
    insert_block(&mut conn, &new_block(2, 700_002, "Pool B"));
    insert_block(&mut conn, &new_block(3, 700_003, "Pool A"));

    let (blocks, max_pages) = db::blocks_by_pool(&mut conn, 0, "Pool A").unwrap();
    assert_eq!(blocks.len(), 2);
    assert_eq!(max_pages, 1);
    // newest block first
    assert_eq!(blocks[0].height, 700_003);
    assert!(blocks.iter().all(|b| b.pool_name == "Pool A"));

    let (blocks, max_pages) = db::blocks_by_pool(&mut conn, 0, "Pool B").unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(max_pages, 1);

    let (blocks, max_pages) = db::blocks_by_pool(&mut conn, 0, "Pool C").unwrap();
    assert!(blocks.is_empty());
    assert_eq!(max_pages, 0);
}

#[test]
fn test_pools() {
    let mut conn = conn_or_skip!();

    insert_block(&mut conn, &new_block(1, 700_001, "Pool B"));
    insert_block(&mut conn, &new_block(2, 700_002, "Pool A"));
    insert_block(&mut conn, &new_block(3, 700_003, "Pool B"));

    // sorted and without duplicates
    assert_eq!(db::pools(&mut conn).unwrap(), vec!["Pool A", "Pool B"]);
}

#[test]
fn test_block_by_hash() {
    let mut conn = conn_or_skip!();

    insert_block(&mut conn, &new_block(1, 700_001, "Pool A"));

    let block = db::block(&hash_n(1), &mut conn).unwrap();
    assert_eq!(block.height, 700_001);
    assert_eq!(block.hash, hash_n(1));

    assert_not_found(db::block(&hash_n(2), &mut conn));
}

#[test]
fn test_unknown_pool_blocks() {
    let mut conn = conn_or_skip!();

    insert_block(&mut conn, &new_block(1, 700_001, "Unknown"));
    insert_block(&mut conn, &new_block(2, 700_002, "Pool A"));
    insert_block(&mut conn, &new_block(3, 700_003, "Unknown"));

    let blocks = db::unknown_pool_blocks(&mut conn).unwrap();
    assert_eq!(blocks.len(), 2);
    // newest block first
    assert_eq!(blocks[0].height, 700_003);
    assert_eq!(blocks[1].height, 700_001);
}

#[test]
fn test_block_with_tx() {
    let mut conn = conn_or_skip!();

    let block_id = insert_block(&mut conn, &new_block(1, 700_001, "Pool A"));
    let other_block_id = insert_block(&mut conn, &new_block(2, 700_002, "Pool A"));

    let txid_a = insert_transaction_n(&mut conn, 10);
    let txid_b = insert_transaction_n(&mut conn, 11);
    let txid_c = insert_transaction_n(&mut conn, 12);

    // inserted out of order: the query sorts by position
    insert_only_in_template(&mut conn, block_id, &txid_b, 5);
    insert_only_in_template(&mut conn, block_id, &txid_a, 1);
    insert_only_in_block(&mut conn, block_id, &txid_c, 3);
    // belongs to another block and must not show up
    insert_only_in_template(&mut conn, other_block_id, &txid_c, 2);

    let block_with_tx = db::block_with_tx(&hash_n(1), &mut conn).unwrap();

    assert_eq!(block_with_tx.block.height, 700_001);
    assert_eq!(block_with_tx.txns_only_in_template.len(), 2);
    assert_eq!(block_with_tx.txns_only_in_template[0].0.position, 1);
    assert_eq!(block_with_tx.txns_only_in_template[0].1.txid, txid_a);
    assert_eq!(block_with_tx.txns_only_in_template[1].0.position, 5);
    assert_eq!(block_with_tx.txns_only_in_template[1].1.txid, txid_b);

    assert_eq!(block_with_tx.txns_only_in_block.len(), 1);
    assert_eq!(block_with_tx.txns_only_in_block[0].0.position, 3);
    assert_eq!(block_with_tx.txns_only_in_block[0].1.txid, txid_c);
}

#[test]
fn test_block_with_tx_for_an_unknown_block() {
    let mut conn = conn_or_skip!();

    assert_not_found(db::block_with_tx(&hash_n(1), &mut conn));
}

//##### CONFLICTING TRANSACTIONS

fn insert_conflict(
    conn: &mut PgConnection,
    block_id: i64,
    template_txids: Vec<Vec<u8>>,
    block_txids: Vec<Vec<u8>>,
    outpoints: Vec<(Vec<u8>, i32)>,
) {
    diesel::insert_into(schema::conflicting_transactions::table)
        .values(ConflictingTransaction {
            block_id,
            template_txids,
            block_txids,
            conflicting_outpoints_txids: outpoints.iter().map(|(t, _)| t.clone()).collect(),
            conflicting_outpoints_vouts: outpoints.iter().map(|(_, v)| *v).collect(),
        })
        .execute(conn)
        .expect("should be able to insert a conflicting_transactions row");
}

/// A block with one conflict between a template and a block transaction.
fn insert_block_with_a_conflict(conn: &mut PgConnection, n: u8, height: i32) -> i64 {
    let block_id = insert_block(conn, &new_block(n, height, "Pool A"));
    let template_txid = insert_transaction_n(conn, n * 10);
    let block_txid = insert_transaction_n(conn, n * 10 + 1);
    insert_conflict(
        conn,
        block_id,
        vec![template_txid],
        vec![block_txid],
        vec![(hash_n(200), 3), (hash_n(201), 7)],
    );
    block_id
}

#[test]
fn test_blocks_with_conflicting_transactions() {
    let mut conn = conn_or_skip!();

    insert_block_with_a_conflict(&mut conn, 1, 700_001);
    insert_block_with_a_conflict(&mut conn, 2, 700_002);
    // a block without a conflict
    insert_block(&mut conn, &new_block(3, 700_003, "Pool A"));

    let (infos, max_pages) = db::blocks_with_conflicting_transactions(&mut conn, 0).unwrap();

    assert_eq!(infos.len(), 2);
    assert_eq!(max_pages, 1);

    let info = infos.iter().find(|i| i.block.height == 700_001).unwrap();
    assert_eq!(info.conflicting_transaction_sets.len(), 1);
    let set = &info.conflicting_transaction_sets[0];
    assert_eq!(set.template_transactions.len(), 1);
    assert_eq!(set.template_transactions[0].txid, hash_n(10));
    assert_eq!(set.block_transactions.len(), 1);
    assert_eq!(set.block_transactions[0].txid, hash_n(11));
    // the outpoint txids and vouts are zipped back together
    assert_eq!(set.conflicting_outpoints.len(), 2);
    assert_eq!(set.conflicting_outpoints[0].txid, hash_n(200));
    assert_eq!(set.conflicting_outpoints[0].vout, 3);
    assert_eq!(set.conflicting_outpoints[1].txid, hash_n(201));
    assert_eq!(set.conflicting_outpoints[1].vout, 7);
}

#[test]
fn test_blocks_with_conflicting_transactions_without_conflicts() {
    let mut conn = conn_or_skip!();

    let (infos, max_pages) = db::blocks_with_conflicting_transactions(&mut conn, 0).unwrap();
    assert!(infos.is_empty());
    assert_eq!(max_pages, 0);
}

#[test]
fn test_single_block_with_conflicting_transactions() {
    let mut conn = conn_or_skip!();

    insert_block_with_a_conflict(&mut conn, 1, 700_001);
    insert_block_with_a_conflict(&mut conn, 2, 700_002);

    let info = db::single_block_with_conflicting_transactions(&mut conn, &hash_n(2)).unwrap();

    assert_eq!(info.block.height, 700_002);
    assert_eq!(info.conflicting_transaction_sets.len(), 1);
    let set = &info.conflicting_transaction_sets[0];
    assert_eq!(set.template_transactions[0].txid, hash_n(20));
    assert_eq!(set.block_transactions[0].txid, hash_n(21));
    assert_eq!(set.conflicting_outpoints.len(), 2);

    assert_not_found(db::single_block_with_conflicting_transactions(
        &mut conn,
        &hash_n(3),
    ));
}

#[test]
fn test_blocks_with_conflicting_transactions_with_an_unknown_transaction() {
    let mut conn = conn_or_skip!();

    // The txids in a conflicting_transactions row are a BYTEA[] without a
    // foreign key, so they can point to a transaction that isn't (or isn't
    // anymore) in the transaction table.
    let block_id = insert_block(&mut conn, &new_block(1, 700_001, "Pool A"));
    let template_txid = insert_transaction_n(&mut conn, 10);
    insert_conflict(
        &mut conn,
        block_id,
        vec![template_txid],
        vec![hash_n(99)],
        vec![(hash_n(200), 3)],
    );

    assert_not_found(db::blocks_with_conflicting_transactions(&mut conn, 0));
    assert_not_found(db::single_block_with_conflicting_transactions(
        &mut conn,
        &hash_n(1),
    ));
}

//##### MISSING TRANSACTIONS

/// Inserts a block a transaction can be missing from. The transaction counts
/// are picked so that a transaction at a low position counts as missing.
fn insert_block_for_missing(conn: &mut PgConnection, n: u8, height: i32) -> i64 {
    let mut block = new_block(n, height, "Pool A");
    block.template_tx = 2000;
    block.block_tx = 1999;
    insert_block(conn, &block)
}

#[test]
fn test_missing_transactions() {
    let mut conn = conn_or_skip!();

    let block_ids: Vec<i64> = (1..=4)
        .map(|n| insert_block_for_missing(&mut conn, n, 700_000 + n as i32))
        .collect();

    // missing from three blocks: is listed
    let missing_txid = insert_transaction_n(&mut conn, 10);
    for block_id in block_ids.iter().take(3) {
        insert_only_in_template(&mut conn, *block_id, &missing_txid, 10);
    }

    // missing from only two blocks: not listed
    let twice_txid = insert_transaction_n(&mut conn, 11);
    for block_id in block_ids.iter().take(2) {
        insert_only_in_template(&mut conn, *block_id, &twice_txid, 10);
    }

    // missing from three blocks, but at the very end of the template, where
    // the template and the block are expected to differ: not listed
    let late_txid = insert_transaction_n(&mut conn, 12);
    for block_id in block_ids.iter().take(3) {
        insert_only_in_template(&mut conn, *block_id, &late_txid, 1999);
    }

    let (missing, max_pages) = db::missing_transactions(&mut conn, 0).unwrap();

    assert_eq!(missing.len(), 1);
    assert_eq!(max_pages, 1);
    let tx = &missing[0];
    assert_eq!(tx.transaction.txid, missing_txid);
    assert_eq!(tx.blocks.len(), 3);
    assert_eq!(tx.max_height(), 700_003);
    // the blocks are sorted by height, newest first
    assert_eq!(
        tx.blocks.iter().map(|b| b.height).collect::<Vec<i32>>(),
        vec![700_003, 700_002, 700_001]
    );
    assert_eq!(tx.blocks[0].pool, "Pool A");
    assert_eq!(tx.blocks[0].template_position, 10);
    assert_eq!(tx.blocks[0].template_tx_count, 2000);
    // the last package feerate of the block fixture
    assert_eq!(tx.blocks[0].last_block_pkg_feerate, 1.0);
}

#[test]
fn test_missing_transactions_are_sorted_by_height() {
    let mut conn = conn_or_skip!();

    let block_ids: Vec<i64> = (1..=6)
        .map(|n| insert_block_for_missing(&mut conn, n, 700_000 + n as i32))
        .collect();

    // missing from the three older blocks
    let older_txid = insert_transaction_n(&mut conn, 10);
    for block_id in block_ids.iter().take(3) {
        insert_only_in_template(&mut conn, *block_id, &older_txid, 10);
    }
    // missing from the three newer blocks
    let newer_txid = insert_transaction_n(&mut conn, 11);
    for block_id in block_ids.iter().skip(3) {
        insert_only_in_template(&mut conn, *block_id, &newer_txid, 10);
    }

    let (missing, _) = db::missing_transactions(&mut conn, 0).unwrap();

    assert_eq!(missing.len(), 2);
    assert_eq!(missing[0].transaction.txid, newer_txid);
    assert_eq!(missing[0].max_height(), 700_006);
    assert_eq!(missing[1].transaction.txid, older_txid);
    assert_eq!(missing[1].max_height(), 700_003);
}

#[test]
fn test_missing_transactions_without_transactions() {
    let mut conn = conn_or_skip!();

    let (missing, max_pages) = db::missing_transactions(&mut conn, 0).unwrap();
    assert!(missing.is_empty());
    assert_eq!(max_pages, 0);
}

#[test]
fn test_single_missing_transaction() {
    let mut conn = conn_or_skip!();

    let block_id_a = insert_block_for_missing(&mut conn, 1, 700_001);
    let block_id_b = insert_block_for_missing(&mut conn, 2, 700_002);

    let txid = insert_transaction_n(&mut conn, 10);
    insert_only_in_template(&mut conn, block_id_a, &txid, 10);
    insert_only_in_template(&mut conn, block_id_b, &txid, 20);

    let missing = db::single_missing_transaction(&txid, &mut conn).unwrap();

    assert_eq!(missing.transaction.txid, txid);
    // the blocks are sorted by height, newest first
    assert_eq!(
        missing
            .blocks
            .iter()
            .map(|b| b.height)
            .collect::<Vec<i32>>(),
        vec![700_002, 700_001]
    );
    assert_eq!(missing.blocks[0].template_position, 20);
    assert_eq!(missing.max_height(), 700_002);

    assert_not_found(db::single_missing_transaction(&hash_n(99), &mut conn));
}

#[test]
fn test_single_missing_transaction_without_packages() {
    let mut conn = conn_or_skip!();

    // a block that has no package feerates
    let mut block = new_block(1, 700_001, "Pool A");
    block.block_pkg_feerates = vec![];
    block.block_pkg_weights = vec![];
    let block_id = insert_block(&mut conn, &block);

    let txid = insert_transaction_n(&mut conn, 10);
    insert_only_in_template(&mut conn, block_id, &txid, 10);

    let missing = db::single_missing_transaction(&txid, &mut conn).unwrap();
    assert_eq!(missing.blocks[0].last_block_pkg_feerate, 0.0);
}

//##### SANCTIONED TRANSACTIONS

fn insert_sanctioned_info(
    conn: &mut PgConnection,
    block_id: i64,
    txid: &[u8],
    in_block: bool,
    in_template: bool,
) {
    diesel::insert_into(schema::sanctioned_transaction_info::table)
        .values(SanctionedTransactionInfo {
            block_id,
            transaction_txid: txid.to_vec(),
            in_block,
            in_template,
            addresses: vec!["1SanctionedAddressxxxxxxxxxxxxxxxx".to_string()],
        })
        .execute(conn)
        .expect("should be able to insert a sanctioned_transaction_info");
}

#[test]
fn test_missing_sanctioned_txns_for_block() {
    let mut conn = conn_or_skip!();

    let block_id = insert_block(&mut conn, &new_block(1, 700_001, "Pool A"));

    let missing_txid = insert_transaction_n(&mut conn, 10);
    insert_only_in_template(&mut conn, block_id, &missing_txid, 42);
    insert_sanctioned_info(&mut conn, block_id, &missing_txid, false, true);

    // a sanctioned transaction that is in the block: not missing
    let included_txid = insert_transaction_n(&mut conn, 11);
    insert_only_in_template(&mut conn, block_id, &included_txid, 43);
    insert_sanctioned_info(&mut conn, block_id, &included_txid, true, true);

    let missing = db::missing_sanctioned_txns_for_block(&hash_n(1), &mut conn).unwrap();

    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].transaction.txid, missing_txid);
    assert_eq!(missing[0].missing_info.position, 42);
    assert_eq!(
        missing[0].addresses,
        vec!["1SanctionedAddressxxxxxxxxxxxxxxxx".to_string()]
    );

    assert_not_found(db::missing_sanctioned_txns_for_block(&hash_n(2), &mut conn));
}

#[test]
fn test_blocks_with_missing_sanctioned() {
    let mut conn = conn_or_skip!();

    let mut with = new_block(1, 700_001, "Pool A");
    with.sanctioned_missing_tx = 2;
    insert_block(&mut conn, &with);

    let mut without = new_block(2, 700_002, "Pool A");
    without.sanctioned_missing_tx = 0;
    insert_block(&mut conn, &without);

    let mut also_with = new_block(3, 700_003, "Pool A");
    also_with.sanctioned_missing_tx = 1;
    insert_block(&mut conn, &also_with);

    let blocks = db::blocks_with_missing_sanctioned(&mut conn).unwrap();

    assert_eq!(blocks.len(), 2);
    // newest block first
    assert_eq!(blocks[0].height, 700_003);
    assert_eq!(blocks[1].height, 700_001);
}

#[test]
fn test_debug_templates_and_blocks_with_sanctioned_tx() {
    let mut conn = conn_or_skip!();

    let mut only_template = new_block(1, 700_001, "Pool A");
    only_template.template_sanctioned = 1;
    only_template.block_sanctioned = 0;
    insert_block(&mut conn, &only_template);

    let mut only_block = new_block(2, 700_002, "Pool A");
    only_block.template_sanctioned = 0;
    only_block.block_sanctioned = 1;
    insert_block(&mut conn, &only_block);

    let mut neither = new_block(3, 700_003, "Pool A");
    neither.template_sanctioned = 0;
    neither.block_sanctioned = 0;
    insert_block(&mut conn, &neither);

    let blocks = db::debug_templates_and_blocks_with_sanctioned_tx(&mut conn).unwrap();

    assert_eq!(blocks.len(), 2);
    let heights: Vec<i32> = blocks.iter().map(|b| b.height).collect();
    // newest block first
    assert_eq!(heights, vec![700_002, 700_001]);
}

#[test]
fn test_debug_sanctioned_table() {
    let mut conn = conn_or_skip!();

    let block_a = insert_block(&mut conn, &new_block(1, 700_001, "Pool A"));
    let block_b = insert_block(&mut conn, &new_block(2, 700_002, "Pool B"));

    let txid_1 = insert_transaction_n(&mut conn, 10);
    let txid_2 = insert_transaction_n(&mut conn, 11);
    let txid_3 = insert_transaction_n(&mut conn, 12);

    // Pool A: one in both, one only in the template
    insert_sanctioned_info(&mut conn, block_a, &txid_1, true, true);
    insert_sanctioned_info(&mut conn, block_a, &txid_2, false, true);
    // Pool B: one only in the block
    insert_sanctioned_info(&mut conn, block_b, &txid_3, true, false);

    let table = db::debug_sanctioned_table(&mut conn).unwrap();

    assert_eq!(table.len(), 2);
    // sorted by pool name
    assert_eq!(table[0].pool_name, "Pool A");
    assert_eq!(table[0].in_both, 1);
    assert_eq!(table[0].only_in_template, 1);
    assert_eq!(table[0].only_in_block, 0);

    assert_eq!(table[1].pool_name, "Pool B");
    assert_eq!(table[1].in_both, 0);
    assert_eq!(table[1].only_in_template, 0);
    assert_eq!(table[1].only_in_block, 1);
}

//##### FEES BY POOL

#[test]
fn test_avg_fees_by_pool() {
    let mut conn = conn_or_skip!();

    // The query only reports pools with more than ten blocks.
    for n in 0..11u8 {
        let mut block = new_block(n, 700_000 + n as i32, "Big Pool");
        block.template_cb_fees = 100;
        block.block_cb_fees = 50 + n as i64;
        insert_block(&mut conn, &block);
    }
    for n in 100..110u8 {
        let mut block = new_block(n, 700_000 + n as i32, "Small Pool");
        block.template_cb_fees = 100;
        block.block_cb_fees = 100;
        insert_block(&mut conn, &block);
    }

    let fees = db::avg_fees_by_pool(&mut conn).unwrap();

    assert_eq!(fees.len(), 1);
    assert_eq!(fees[0].pool_name, "Big Pool");
    assert_eq!(fees[0].count, 11);
    // block fees from 50 to 60 out of 100 template fees
    assert_eq!(fees[0].median, 0.55);
    assert!(fees[0].q1 < fees[0].median);
    assert!(fees[0].q3 > fees[0].median);
}

//##### UTXO SET SCANS

fn insert_scans(conn: &mut PgConnection, count: i64) {
    let scans: Vec<SanctionedUtxoScanInfo> = (0..count)
        .map(|i| SanctionedUtxoScanInfo {
            end_time: time_plus(i),
            end_height: 700_000 + i as i32,
            duration_seconds: i as i32,
            utxo_amount: 1000 + i,
            utxo_count: i as i32,
        })
        .collect();
    diesel::insert_into(schema::sanctioned_utxo_scan_info::table)
        .values(&scans)
        .execute(conn)
        .expect("should be able to insert the scans");
}

#[test]
fn test_sanctioned_utxo_scan_infos_pagination() {
    let mut conn = conn_or_skip!();

    let count = db::MAX_UTXOSET_SCANS_PER_PAGE + 1;
    insert_scans(&mut conn, count);

    let (first_page, max_pages) = db::sanctioned_utxo_scan_infos(&mut conn, 0).unwrap();
    assert_eq!(first_page.len(), db::MAX_UTXOSET_SCANS_PER_PAGE as usize);
    assert_eq!(max_pages, 2);
    // newest scan first
    assert_eq!(first_page[0].end_height, 700_000 + count as i32 - 1);

    let (second_page, _) = db::sanctioned_utxo_scan_infos(&mut conn, 1).unwrap();
    assert_eq!(second_page.len(), 1);
    assert_eq!(second_page[0].end_height, 700_000);
}

#[test]
fn test_get_recent_sanctioned_utxo_scan_info() {
    let mut conn = conn_or_skip!();

    assert_not_found(db::get_recent_sanctioned_utxo_scan_info(&mut conn));

    insert_scans(&mut conn, 3);

    let recent = db::get_recent_sanctioned_utxo_scan_info(&mut conn).unwrap();
    assert_eq!(recent.end_height, 700_002);
    assert_eq!(recent.end_time, time_plus(2));
}

//##### TEMPLATE SELECTION

#[test]
fn test_debug_template_selection_infos() {
    let mut conn = conn_or_skip!();

    let block_id = insert_block(&mut conn, &new_block(1, 700_001, "Pool A"));
    // a block without template selection infos
    insert_block(&mut conn, &new_block(2, 700_002, "Pool A"));

    for (i, selected) in [false, true, false].iter().enumerate() {
        let mut info = debug_template_selection_info_fixture(*selected);
        info.block_id = block_id;
        info.template_time = time_plus(i as i64);
        diesel::insert_into(schema::debug_template_selection::table)
            .values(info)
            .execute(&mut conn)
            .expect("should be able to insert a debug_template_selection");
    }

    let (infos_and_blocks, max_pages) = db::debug_template_selection_infos(&mut conn, 0).unwrap();

    assert_eq!(infos_and_blocks.len(), 1);
    assert_eq!(max_pages, 1);
    assert_eq!(infos_and_blocks[0].block.height, 700_001);
    assert_eq!(infos_and_blocks[0].infos.len(), 3);
    // the newest template first
    assert_eq!(infos_and_blocks[0].infos[0].template_time, time_plus(2));
    assert!(infos_and_blocks[0].infos.iter().any(|i| i.selected));
}

#[test]
fn test_debug_template_selection_infos_without_infos() {
    let mut conn = conn_or_skip!();

    let (infos_and_blocks, max_pages) = db::debug_template_selection_infos(&mut conn, 0).unwrap();
    assert!(infos_and_blocks.is_empty());
    assert_eq!(max_pages, 0);
}

//##### NODE INFO AND SANCTIONED ADDRESSES

#[test]
fn test_get_node_info() {
    let mut conn = conn_or_skip!();

    // the migrations insert a placeholder the daemon replaces
    assert_eq!(db::get_node_info(&mut conn).unwrap(), "-");

    diesel::update(schema::node_info::table)
        .set(schema::node_info::version.eq("/Satoshi:99.0.0/"))
        .execute(&mut conn)
        .expect("should be able to update the node info");

    assert_eq!(db::get_node_info(&mut conn).unwrap(), "/Satoshi:99.0.0/");
}

#[test]
fn test_sanctioned_addresses() {
    let mut conn = conn_or_skip!();

    assert!(db::sanctioned_addresses(&mut conn).unwrap().is_empty());

    let addresses = vec![
        SanctionedAddress {
            address: "1Sanctioned".to_string(),
        },
        SanctionedAddress {
            address: "3Sanctioned".to_string(),
        },
    ];
    diesel::insert_into(schema::sanctioned_addresses::table)
        .values(&addresses)
        .execute(&mut conn)
        .expect("should be able to insert the sanctioned addresses");

    let mut loaded = db::sanctioned_addresses(&mut conn).unwrap();
    loaded.sort();
    assert_eq!(loaded, vec!["1Sanctioned", "3Sanctioned"]);
}
