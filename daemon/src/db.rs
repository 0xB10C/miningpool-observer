use std::collections::HashSet;
use std::error::Error;
use std::iter::FromIterator;

use miningpool_observer_shared::model::{
    Block, ConflictingTransaction, DebugTemplateSelectionInfo, NewBlock, SanctionedTransactionInfo,
    SanctionedUtxo, SanctionedUtxoScanInfo, Transaction, TransactionOnlyInBlock,
    TransactionOnlyInTemplate,
};
use miningpool_observer_shared::schema;

use diesel::pg::PgConnection;
use diesel::prelude::*;
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("../migrations/");

pub fn run_migrations(
    conn: &mut PgConnection,
) -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    conn.run_pending_migrations(MIGRATIONS)?;
    Ok(())
}

/// Insert a single block
pub fn insert_block(b: &NewBlock, conn: &mut PgConnection) -> Result<i32, diesel::result::Error> {
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

pub fn all_transactions(
    conn: &mut PgConnection,
) -> Result<Vec<Transaction>, diesel::result::Error> {
    use schema::transaction::dsl::*;
    transaction.load::<Transaction>(conn)
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
    block_id: i32,
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
