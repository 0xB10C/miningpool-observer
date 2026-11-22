use std::collections::HashSet;
use std::error::Error;
use std::iter::FromIterator;

use miningpool_observer_shared::model::{
    Block, ConflictingTransaction, DebugTemplateSelectionInfo, NewBlock, SanctionedAddress,
    SanctionedTransactionInfo, SanctionedUtxo, SanctionedUtxoScanInfo, Transaction,
    TransactionOnlyInBlock, TransactionOnlyInTemplate,
};
use miningpool_observer_shared::schema;

use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use miningpool_observer_shared::diesel;
use miningpool_observer_shared::diesel::pg::PgConnection;
use miningpool_observer_shared::diesel::prelude::*;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("../migrations/");

pub fn run_migrations(
    conn: &mut PgConnection,
) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    conn.run_pending_migrations(MIGRATIONS)?;
    Ok(())
}

/// Insert a single block
pub fn insert_block(b: &NewBlock, conn: &mut PgConnection) -> Result<i64, diesel::result::Error> {
    use schema::block::dsl::*;
    diesel::insert_into(block)
        .values(b)
        .returning(id)
        .get_result(conn)
}

pub fn insert_transactions(
    txns: Vec<Transaction>,
    conn: &mut PgConnection,
) -> Result<(), diesel::result::Error> {
    use schema::transaction::dsl::*;
    let inserted_txids = diesel::insert_into(transaction)
        .values(&txns)
        .on_conflict_do_nothing()
        .returning(txid)
        .get_results::<Vec<u8>>(conn)?;

    let inserted_txids_hashset: HashSet<Vec<u8>> = inserted_txids.iter().cloned().collect();
    let txns_txids_hashset: HashSet<Vec<u8>> = txns.iter().map(|tx| tx.txid.clone()).collect();

    // check if there were any conflicts, if yes, then check on which txids
    // and try to merge the tag arrays of the new and the already-present transactions
    if inserted_txids_hashset.len() != txns_txids_hashset.len() {
        log::info!(target: "db", "While inserting {} transactions, {} were already present in the database.", txns_txids_hashset.len(), txns_txids_hashset.len() - inserted_txids_hashset.len());

        let mut updated_count: u32 = 0;
        for tx in txns.iter() {
            if !inserted_txids_hashset.contains(&tx.txid) {
                // load the transactions tags from the database
                let db_tx_tags: Vec<i32> = transaction
                    .select(tags)
                    .filter(txid.eq(&tx.txid))
                    .first(conn)?;

                if db_tx_tags != tx.tags {
                    updated_count += 1;
                    let both_tags: HashSet<&i32> =
                        db_tx_tags.iter().chain(tx.tags.iter()).collect();
                    let mut both_tags_vec = Vec::from_iter(both_tags);
                    both_tags_vec.sort();
                    // update the transaction in the database with the combined tags
                    diesel::update(transaction.filter(txid.eq(&tx.txid)))
                        .set(tags.eq(both_tags_vec))
                        .execute(conn)?;
                }
            }
        }
        log::info!(target: "db", "Checked the tags of {} transactions. Merged and updated tags of {} transactions.", txns.len() - inserted_txids_hashset.len(), updated_count);
    }

    Ok(())
}

pub fn insert_transactions_only_in_block(
    txns: Vec<TransactionOnlyInBlock>,
    conn: &mut PgConnection,
) -> Result<(), diesel::result::Error> {
    use schema::transaction_only_in_block::dsl::*;
    diesel::insert_into(transaction_only_in_block)
        .values(txns)
        .execute(conn)?;
    Ok(())
}

pub fn insert_transactions_only_in_template(
    txns: Vec<TransactionOnlyInTemplate>,
    conn: &mut PgConnection,
) -> Result<(), diesel::result::Error> {
    use schema::transaction_only_in_template::dsl::*;
    diesel::insert_into(transaction_only_in_template)
        .values(txns)
        .execute(conn)?;
    Ok(())
}

pub fn insert_sanctioned_transaction_infos(
    sanctioned_infos: Vec<SanctionedTransactionInfo>,
    conn: &mut PgConnection,
) -> Result<(), diesel::result::Error> {
    use schema::sanctioned_transaction_info::dsl::*;
    diesel::insert_into(sanctioned_transaction_info)
        .values(sanctioned_infos)
        .execute(conn)?;
    Ok(())
}

pub fn insert_conflicting_transactions(
    ctxns: Vec<ConflictingTransaction>,
    conn: &mut PgConnection,
) -> Result<(), diesel::result::Error> {
    use schema::conflicting_transactions::dsl::*;
    diesel::insert_into(conflicting_transactions)
        .values(ctxns)
        .execute(conn)?;
    Ok(())
}

/// Deletes all Sanctioned UTXOs and inserts the passed Sanctioned UTXOs.
pub fn clean_and_insert_sanctioned_utxos(
    utxos: &[SanctionedUtxo],
    conn: &mut PgConnection,
) -> Result<(), diesel::result::Error> {
    conn.transaction::<_, diesel::result::Error, _>(|conn| {
        use schema::sanctioned_utxo::dsl::*;
        diesel::delete(sanctioned_utxo).execute(conn)?;
        diesel::insert_into(sanctioned_utxo)
            .values(utxos)
            .execute(conn)?;
        Ok(())
    })?;
    Ok(())
}

pub fn insert_sanctioned_utxos(
    utxos: &[SanctionedUtxo],
    conn: &mut PgConnection,
) -> Result<(), diesel::result::Error> {
    use schema::sanctioned_utxo::dsl::*;
    diesel::insert_into(sanctioned_utxo)
        .values(utxos)
        .on_conflict_do_nothing()
        .execute(conn)?;
    Ok(())
}

pub fn insert_sanctioned_utxo_scan_info(
    info: &SanctionedUtxoScanInfo,
    conn: &mut PgConnection,
) -> Result<(), diesel::result::Error> {
    use schema::sanctioned_utxo_scan_info::dsl::*;
    diesel::insert_into(sanctioned_utxo_scan_info)
        .values(info)
        .on_conflict_do_nothing()
        .execute(conn)?;
    Ok(())
}

pub fn get_sanctioned_utxos(
    conn: &mut PgConnection,
) -> Result<Vec<SanctionedUtxo>, diesel::result::Error> {
    use schema::sanctioned_utxo::dsl::*;
    let utxos = sanctioned_utxo.get_results(conn)?;
    Ok(utxos)
}

pub fn insert_debug_template_selection_infos(
    infos: Vec<DebugTemplateSelectionInfo>,
    conn: &mut PgConnection,
) -> Result<(), diesel::result::Error> {
    use schema::debug_template_selection::dsl::*;
    diesel::insert_into(debug_template_selection)
        .values(infos)
        .on_conflict_do_nothing()
        .execute(conn)?;
    Ok(())
}

pub fn replace_sanctioned_addresses(
    addr: Vec<SanctionedAddress>,
    conn: &mut PgConnection,
) -> Result<(), diesel::result::Error> {
    conn.transaction::<_, diesel::result::Error, _>(|conn| {
        use schema::sanctioned_addresses::dsl::*;
        diesel::delete(sanctioned_addresses).execute(conn)?;
        diesel::insert_into(sanctioned_addresses)
            .values(addr)
            .execute(conn)?;
        Ok(())
    })?;
    Ok(())
}

pub fn all_transactions(
    conn: &mut PgConnection,
) -> Result<Vec<Transaction>, diesel::result::Error> {
    use schema::transaction::dsl::*;
    transaction.load::<Transaction>(conn)
}

pub fn sanctioned_addresses(
    conn: &mut PgConnection,
) -> Result<Vec<SanctionedAddress>, diesel::result::Error> {
    use schema::sanctioned_addresses::dsl::*;
    sanctioned_addresses.load::<SanctionedAddress>(conn)
}

pub fn unknown_pool_blocks(conn: &mut PgConnection) -> Result<Vec<Block>, diesel::result::Error> {
    use schema::block::dsl::*;
    block
        .filter(pool_name.eq("Unknown"))
        .order(block_time.desc())
        .load::<Block>(conn)
}

pub fn update_pool_name_with_block_id(
    conn: &mut PgConnection,
    block_id: i64,
    new_pool_name: &str,
) -> Result<(), diesel::result::Error> {
    use schema::block::dsl::*;

    diesel::update(block.filter(id.eq(block_id)))
        .set(pool_name.eq(new_pool_name))
        .execute(conn)?;
    Ok(())
}

pub fn update_transaction_tags(
    new_tags: &Vec<i32>,
    tx_id: &Vec<u8>,
    conn: &mut PgConnection,
) -> Result<(), diesel::result::Error> {
    use schema::transaction::dsl::*;
    diesel::update(transaction)
        .filter(txid.eq(tx_id))
        .set(tags.eq(new_tags))
        .execute(conn)?;
    Ok(())
}

/// Update node information (e.g. node version)
pub fn update_node_info(
    new_version: &str,
    conn: &mut PgConnection,
) -> Result<(), diesel::result::Error> {
    use schema::node_info::dsl::*;
    diesel::update(node_info.filter(id.eq(0)))
        .set(version.eq(new_version))
        .execute(conn)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{db_transaction_fixture, new_block_fixture, test_conn};
    use miningpool_observer_shared::chrono;

    /// Runs the body of a database test with a connection, or prints why it
    /// was skipped and returns.
    macro_rules! db_test {
        ($conn:ident, $body:block) => {
            match test_conn() {
                Some(mut $conn) => $body,
                None => return,
            }
        };
    }

    fn insert_test_block(hash_byte: u8, conn: &mut PgConnection) -> i64 {
        insert_block(&new_block_fixture(hash_byte, 800_000), conn).unwrap()
    }

    fn transaction_in_db(tx_id: &[u8], conn: &mut PgConnection) -> Transaction {
        use schema::transaction::dsl::*;
        transaction
            .filter(txid.eq(tx_id))
            .first::<Transaction>(conn)
            .unwrap()
    }

    #[test]
    fn test_insert_block() {
        db_test!(conn, {
            let block_id = insert_block(&new_block_fixture(0x11, 800_000), &mut conn).unwrap();
            assert!(block_id > 0);

            // the block hash is the primary key, so the same block can't be
            // inserted twice
            assert!(insert_block(&new_block_fixture(0x11, 800_000), &mut conn).is_err());
        })
    }

    #[test]
    fn test_insert_transactions_merges_the_tags() {
        db_test!(conn, {
            let tx = db_transaction_fixture(0x21, vec![4110, 4140]);
            insert_transactions(vec![tx.clone()], &mut conn).unwrap();
            assert_eq!(
                transaction_in_db(&tx.txid, &mut conn).tags,
                vec![4110, 4140]
            );

            // inserting the same transaction with different tags merges them
            let mut with_other_tags = tx.clone();
            with_other_tags.tags = vec![4100, 4110];
            insert_transactions(vec![with_other_tags], &mut conn).unwrap();
            assert_eq!(
                transaction_in_db(&tx.txid, &mut conn).tags,
                vec![4100, 4110, 4140]
            );

            // inserting it with the tags it already has changes nothing
            let mut with_same_tags = tx.clone();
            with_same_tags.tags = vec![4100, 4110, 4140];
            insert_transactions(vec![with_same_tags], &mut conn).unwrap();
            assert_eq!(
                transaction_in_db(&tx.txid, &mut conn).tags,
                vec![4100, 4110, 4140]
            );

            let all = all_transactions(&mut conn).unwrap();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0].txid, tx.txid);
            assert_eq!(all[0].fee, 1_000);
            assert_eq!(all[0].vsize, 250);
        })
    }

    #[test]
    fn test_update_transaction_tags() {
        db_test!(conn, {
            let tx = db_transaction_fixture(0x22, vec![4110]);
            insert_transactions(vec![tx.clone()], &mut conn).unwrap();

            update_transaction_tags(&vec![1100, 4110], &tx.txid, &mut conn).unwrap();
            assert_eq!(
                transaction_in_db(&tx.txid, &mut conn).tags,
                vec![1100, 4110]
            );
        })
    }

    #[test]
    fn test_replace_sanctioned_addresses() {
        db_test!(conn, {
            let old = vec![SanctionedAddress {
                address: "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2".to_string(),
            }];
            replace_sanctioned_addresses(old, &mut conn).unwrap();
            assert_eq!(sanctioned_addresses(&mut conn).unwrap().len(), 1);

            let new = vec![
                SanctionedAddress {
                    address: "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_string(),
                },
                SanctionedAddress {
                    address: "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy".to_string(),
                },
            ];
            replace_sanctioned_addresses(new, &mut conn).unwrap();

            let mut addresses: Vec<String> = sanctioned_addresses(&mut conn)
                .unwrap()
                .iter()
                .map(|a| a.address.clone())
                .collect();
            addresses.sort();
            assert_eq!(
                addresses,
                vec![
                    "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy".to_string(),
                    "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4".to_string(),
                ]
            );
        })
    }

    fn sanctioned_utxo_fixture(txid_byte: u8, vout: i32, amount: i64) -> SanctionedUtxo {
        SanctionedUtxo {
            txid: vec![txid_byte; 32],
            vout,
            script_pubkey: vec![0x00, 0x14, 0x42],
            amount,
            height: 800_000,
        }
    }

    #[test]
    fn test_sanctioned_utxos() {
        db_test!(conn, {
            clean_and_insert_sanctioned_utxos(
                &[sanctioned_utxo_fixture(0x31, 0, 1_000)],
                &mut conn,
            )
            .unwrap();
            assert_eq!(get_sanctioned_utxos(&mut conn).unwrap().len(), 1);

            // inserting a UTXO that is already there is ignored
            insert_sanctioned_utxos(&[sanctioned_utxo_fixture(0x31, 0, 1_000)], &mut conn).unwrap();
            assert_eq!(get_sanctioned_utxos(&mut conn).unwrap().len(), 1);

            // the same txid with a different vout is a different UTXO
            let second = sanctioned_utxo_fixture(0x31, 1, 2_000);
            insert_sanctioned_utxos(&[second], &mut conn).unwrap();
            assert_eq!(get_sanctioned_utxos(&mut conn).unwrap().len(), 2);

            // cleaning and inserting replaces all of them
            let third = sanctioned_utxo_fixture(0x32, 0, 3_000);
            clean_and_insert_sanctioned_utxos(&[third], &mut conn).unwrap();
            let utxos = get_sanctioned_utxos(&mut conn).unwrap();
            assert_eq!(utxos.len(), 1);
            assert_eq!(utxos[0].txid, vec![0x32; 32]);
            assert_eq!(utxos[0].amount, 3_000);
            assert_eq!(utxos[0].height, 800_000);
        })
    }

    #[test]
    fn test_insert_sanctioned_utxo_scan_info() {
        db_test!(conn, {
            let info = SanctionedUtxoScanInfo {
                end_time: chrono::DateTime::from_timestamp(1_600_000_000, 0)
                    .unwrap()
                    .naive_utc(),
                end_height: 800_000,
                duration_seconds: 42,
                utxo_amount: 1_000,
                utxo_count: 7,
            };
            insert_sanctioned_utxo_scan_info(&info, &mut conn).unwrap();
            // the end time is the primary key, inserting it again is ignored
            insert_sanctioned_utxo_scan_info(&info, &mut conn).unwrap();

            use schema::sanctioned_utxo_scan_info::dsl::*;
            let infos = sanctioned_utxo_scan_info
                .load::<SanctionedUtxoScanInfo>(&mut conn)
                .unwrap();
            assert_eq!(infos.len(), 1);
            assert_eq!(infos[0].utxo_count, 7);
        })
    }

    #[test]
    fn test_insert_debug_template_selection_infos() {
        db_test!(conn, {
            let db_block_id = insert_test_block(0x41, &mut conn);
            let info = DebugTemplateSelectionInfo {
                block_id: db_block_id,
                template_time: chrono::DateTime::from_timestamp(1_600_000_000, 0)
                    .unwrap()
                    .naive_utc(),
                count_missing: 1,
                count_shared: 2,
                count_extra: 3,
                selected: true,
            };
            insert_debug_template_selection_infos(vec![info.clone()], &mut conn).unwrap();
            // the block id and template time are the primary key
            insert_debug_template_selection_infos(vec![info], &mut conn).unwrap();

            use schema::debug_template_selection::dsl::*;
            let infos = debug_template_selection
                .load::<DebugTemplateSelectionInfo>(&mut conn)
                .unwrap();
            assert_eq!(infos.len(), 1);
            assert_eq!(infos[0].count_shared, 2);
        })
    }

    #[test]
    fn test_insert_transactions_only_in_block_and_template() {
        db_test!(conn, {
            let db_block_id = insert_test_block(0x51, &mut conn);
            let in_block = db_transaction_fixture(0x52, vec![]);
            let in_template = db_transaction_fixture(0x53, vec![]);
            insert_transactions(vec![in_block.clone(), in_template.clone()], &mut conn).unwrap();

            insert_transactions_only_in_block(
                vec![TransactionOnlyInBlock {
                    block_id: db_block_id,
                    position: 3,
                    transaction_txid: in_block.txid.clone(),
                }],
                &mut conn,
            )
            .unwrap();
            insert_transactions_only_in_template(
                vec![TransactionOnlyInTemplate {
                    block_id: db_block_id,
                    position: 4,
                    mempool_age_seconds: 120,
                    transaction_txid: in_template.txid.clone(),
                }],
                &mut conn,
            )
            .unwrap();

            use schema::transaction_only_in_block::dsl as block_dsl;
            let only_in_block = block_dsl::transaction_only_in_block
                .load::<TransactionOnlyInBlock>(&mut conn)
                .unwrap();
            assert_eq!(only_in_block.len(), 1);
            assert_eq!(only_in_block[0].position, 3);
            assert_eq!(only_in_block[0].transaction_txid, in_block.txid);

            use schema::transaction_only_in_template::dsl as template_dsl;
            let only_in_template = template_dsl::transaction_only_in_template
                .load::<TransactionOnlyInTemplate>(&mut conn)
                .unwrap();
            assert_eq!(only_in_template.len(), 1);
            assert_eq!(only_in_template[0].position, 4);
            assert_eq!(only_in_template[0].mempool_age_seconds, 120);
        })
    }

    #[test]
    fn test_insert_sanctioned_transaction_infos() {
        db_test!(conn, {
            let db_block_id = insert_test_block(0x61, &mut conn);
            let tx = db_transaction_fixture(0x62, vec![]);
            insert_transactions(vec![tx.clone()], &mut conn).unwrap();

            insert_sanctioned_transaction_infos(
                vec![SanctionedTransactionInfo {
                    block_id: db_block_id,
                    transaction_txid: tx.txid.clone(),
                    in_block: true,
                    in_template: false,
                    addresses: vec!["1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2".to_string()],
                }],
                &mut conn,
            )
            .unwrap();

            use schema::sanctioned_transaction_info::dsl::*;
            let infos = sanctioned_transaction_info
                .load::<SanctionedTransactionInfo>(&mut conn)
                .unwrap();
            assert_eq!(infos.len(), 1);
            assert!(infos[0].in_block);
            assert!(!infos[0].in_template);
            assert_eq!(
                infos[0].addresses,
                vec!["1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2".to_string()]
            );
        })
    }

    #[test]
    fn test_insert_conflicting_transactions() {
        db_test!(conn, {
            let db_block_id = insert_test_block(0x71, &mut conn);

            insert_conflicting_transactions(
                vec![ConflictingTransaction {
                    block_id: db_block_id,
                    template_txids: vec![vec![0x72; 32]],
                    block_txids: vec![vec![0x73; 32]],
                    conflicting_outpoints_txids: vec![vec![0x74; 32]],
                    conflicting_outpoints_vouts: vec![1],
                }],
                &mut conn,
            )
            .unwrap();

            use schema::conflicting_transactions::dsl::*;
            let conflicts = conflicting_transactions
                .load::<ConflictingTransaction>(&mut conn)
                .unwrap();
            assert_eq!(conflicts.len(), 1);
            assert_eq!(conflicts[0].template_txids, vec![vec![0x72; 32]]);
            assert_eq!(conflicts[0].conflicting_outpoints_vouts, vec![1]);
        })
    }

    #[test]
    fn test_unknown_pool_blocks_and_update_pool_name() {
        db_test!(conn, {
            let db_block_id = insert_test_block(0x81, &mut conn);

            let unknown = unknown_pool_blocks(&mut conn).unwrap();
            assert_eq!(unknown.len(), 1);
            assert_eq!(unknown[0].id, db_block_id);
            assert_eq!(unknown[0].pool_name, "Unknown");

            update_pool_name_with_block_id(&mut conn, db_block_id, "Test Pool").unwrap();
            assert!(unknown_pool_blocks(&mut conn).unwrap().is_empty());

            use schema::block::dsl;
            let updated = dsl::block
                .filter(dsl::id.eq(db_block_id))
                .first::<Block>(&mut conn)
                .unwrap();
            assert_eq!(updated.pool_name, "Test Pool");
        })
    }

    #[test]
    fn test_update_node_info() {
        db_test!(conn, {
            update_node_info("Satoshi 27.0.0", &mut conn).unwrap();

            use schema::node_info::dsl::*;
            let stored: String = node_info
                .select(version)
                .filter(id.eq(0))
                .first(&mut conn)
                .unwrap();
            assert_eq!(stored, "Satoshi 27.0.0");
        })
    }
}
