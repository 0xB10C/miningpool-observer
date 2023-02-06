use crate::model::{
    AvgPoolFees, BlockWithTx, ConflictingOutpoint, ConflictingTransactionSet,
    ConflictingTranscationInfo, DebugTemplateSelectionInfosAndBlock, MissingSanctionedTransaction,
    MissingTransaction, MissingTransactionBlockInfo, PoolSanctionedTableEntry,
};
use miningpool_observer_shared::model::{
    Block, ConflictingTransaction, DebugTemplateSelectionInfo, SanctionedTransactionInfo,
    SanctionedUtxoScanInfo, Transaction, TransactionOnlyInBlock, TransactionOnlyInTemplate,
};
use miningpool_observer_shared::schema;

use diesel::dsl::{count, sql};
use diesel::pg::PgConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bytea};
use diesel::{sql_query, QueryableByName};

use std::collections::HashMap;

pub const MAX_BLOCKS_PER_PAGE: i64 = 25;
pub const MAX_UTXOSET_SCANS_PER_PAGE: i64 = 100;

pub fn blocks(
    conn: &mut PgConnection,
    page: u32,
) -> Result<(Vec<Block>, u32), diesel::result::Error> {
    use schema::block::dsl::*;
    let blocks: Vec<Block> = block
        .limit(MAX_BLOCKS_PER_PAGE)
        .offset(MAX_BLOCKS_PER_PAGE * page as i64)
        .order(block_time.desc())
        .load::<Block>(conn)?;

    let blocks_count: i64 = *block.select(count(id)).load(conn)?.first().unwrap();
    let max_pages: u32 = (blocks_count as f32 / MAX_BLOCKS_PER_PAGE as f32).ceil() as u32;
    Ok((blocks, max_pages))
}

pub fn blocks_by_pool(
    conn: &mut PgConnection,
    page: u32,
    pool: &str,
) -> Result<(Vec<Block>, u32), diesel::result::Error> {
    use schema::block::dsl::*;
    let blocks: Vec<Block> = block
        .filter(pool_name.eq(pool))
        .limit(MAX_BLOCKS_PER_PAGE)
        .offset(MAX_BLOCKS_PER_PAGE * page as i64)
        .order(block_time.desc())
        .load::<Block>(conn)?;

    let blocks_count: i64 = *block
        .select(count(id))
        .filter(pool_name.eq(pool))
        .load(conn)?
        .first()
        .unwrap();
    let max_pages: u32 = (blocks_count as f32 / MAX_BLOCKS_PER_PAGE as f32).ceil() as u32;
    Ok((blocks, max_pages))
}

pub fn pools(conn: &mut PgConnection) -> Result<Vec<String>, diesel::result::Error> {
    use schema::block::dsl::*;
    let pools: Vec<String> = block
        .select(pool_name)
        .distinct()
        .order(pool_name.asc())
        .load::<String>(conn)?;
    Ok(pools)
}

/// Select block by hash
pub fn block(block_hash: &[u8], conn: &mut PgConnection) -> Result<Block, diesel::result::Error> {
    use schema::block::dsl::*;
    block.find(block_hash).first(conn)
}

pub fn unknown_pool_blocks(conn: &mut PgConnection) -> Result<Vec<Block>, diesel::result::Error> {
    use schema::block::dsl::*;
    block
        .filter(pool_name.eq("Unknown"))
        .order(block_time.desc())
        .load::<Block>(conn)
}

pub fn block_with_tx(
    block_hash: &[u8],
    conn: &mut PgConnection,
) -> Result<BlockWithTx, diesel::result::Error> {
    let block = block(block_hash, conn)?;
    let block_id = block.id;
    Ok(BlockWithTx {
        block,
        txns_only_in_template: transaction_only_in_template_by_block_id(block_id, conn)?,
        txns_only_in_block: transaction_only_in_block_by_block_id(block_id, conn)?,
    })
}

struct ConflictingTransactionsData {
    blocks: Vec<Block>,
    transactions: Vec<Transaction>,
    conflicting_transactions_info: Vec<ConflictingTransaction>,
    total_block_count: i64,
}

fn conflicting_transactions_data(
    conn: &mut PgConnection,
    page: u32,
) -> Result<ConflictingTransactionsData, diesel::result::Error> {
    let block_ids: Vec<i32>;
    let total_block_count: i64;
    {
        use schema::block::dsl::*;
        use schema::conflicting_transactions::dsl::*;
        block_ids = conflicting_transactions
            .inner_join(block.on(id.eq(block_id)))
            .select((block_id, height))
            .limit(MAX_BLOCKS_PER_PAGE)
            .offset(MAX_BLOCKS_PER_PAGE * page as i64)
            .distinct()
            .order(height.desc())
            .load::<(i32, i32)>(conn)?
            .iter()
            .map(|(a, _)| *a)
            .collect::<Vec<i32>>();
        // SQL: DISTINCT needs the height in the SELECT when it's used in to ORDER BY height.
        // We don't need the height.

        total_block_count = *conflicting_transactions
            .select(sql::<BigInt>("COUNT (DISTINCT block_id)"))
            .load::<i64>(conn)?
            .first()
            .unwrap();
    }

    let conflicting_transactions_info: Vec<ConflictingTransaction>;
    {
        use schema::conflicting_transactions::dsl::*;
        conflicting_transactions_info = conflicting_transactions
            .filter(block_id.eq_any(block_ids.clone()))
            .load::<ConflictingTransaction>(conn)?;
    }

    let mut txids: Vec<&Vec<u8>> = vec![];

    for cti in conflicting_transactions_info.iter() {
        for txid in cti.template_txids.iter() {
            txids.push(txid);
        }
        for txid in cti.block_txids.iter() {
            txids.push(txid);
        }
    }

    let transactions: Vec<Transaction>;
    {
        use schema::transaction::dsl::*;
        transactions = transaction
            .filter(txid.eq_any(txids))
            .load::<Transaction>(conn)?;
    }

    let blocks: Vec<Block>;
    {
        use schema::block::dsl::*;
        blocks = block
            .filter(id.eq_any(block_ids))
            .order(id.desc())
            .load::<Block>(conn)?;
    }

    Ok(ConflictingTransactionsData {
        blocks,
        transactions,
        conflicting_transactions_info,
        total_block_count,
    })
}

pub fn blocks_with_conflicting_transactions(
    conn: &mut PgConnection,
    page: u32,
) -> Result<(Vec<ConflictingTranscationInfo>, u32), diesel::result::Error> {
    let data = conflicting_transactions_data(conn, page)?;
    let mut blocks_with_conflicting_transctions: Vec<ConflictingTranscationInfo> = vec![];
    let txid_to_tx_map: HashMap<Vec<u8>, &Transaction> = data
        .transactions
        .iter()
        .map(|tx| (tx.txid.clone(), tx))
        .collect();

    for block in data.blocks {
        let ct = data
            .conflicting_transactions_info
            .iter()
            .filter(|ct| ct.block_id == block.id);
        blocks_with_conflicting_transctions.push(ConflictingTranscationInfo {
            conflicting_transaction_sets: ct
                .map(|ct| ConflictingTransactionSet {
                    template_transactions: ct
                        .template_txids
                        .iter()
                        .map(|txid| (*txid_to_tx_map.get(txid).unwrap()).clone())
                        .collect(),
                    block_transactions: ct
                        .block_txids
                        .iter()
                        .map(|txid| (*txid_to_tx_map.get(txid).unwrap()).clone())
                        .collect(),
                    conflicting_outpoints: ct
                        .conflicting_outpoints_txids
                        .iter()
                        .zip(&ct.conflicting_outpoints_vouts)
                        .map(|(txid, vout)| ConflictingOutpoint {
                            txid: txid.clone(),
                            vout: *vout,
                        })
                        .collect(),
                })
                .collect(),
            block,
        })
    }

    let max_pages: u32 = (data.total_block_count as f32 / MAX_BLOCKS_PER_PAGE as f32).ceil() as u32;
    Ok((blocks_with_conflicting_transctions, max_pages))
}

fn single_block_with_conflicting_transactions_data(
    conn: &mut PgConnection,
    block_hash: &[u8],
) -> Result<(Block, Vec<Transaction>, Vec<ConflictingTransaction>), diesel::result::Error> {
    let this_block: Block;
    {
        use schema::block::dsl::*;
        this_block = block.filter(hash.eq(block_hash)).first(conn)?;
    }

    let conflicting_transactions_info: Vec<ConflictingTransaction>;
    {
        use schema::conflicting_transactions::dsl::*;
        conflicting_transactions_info = conflicting_transactions
            .filter(block_id.eq(this_block.id))
            .load::<ConflictingTransaction>(conn)?;
    }

    let mut txids: Vec<&Vec<u8>> = vec![];
    for cti in conflicting_transactions_info.iter() {
        for txid in cti.template_txids.iter() {
            txids.push(txid);
        }
        for txid in cti.block_txids.iter() {
            txids.push(txid);
        }
    }

    let transactions: Vec<Transaction>;
    {
        use schema::transaction::dsl::*;
        transactions = transaction
            .filter(txid.eq_any(txids))
            .load::<Transaction>(conn)?;
    }

    Ok((this_block, transactions, conflicting_transactions_info))
}

pub fn single_block_with_conflicting_transactions(
    conn: &mut PgConnection,
    block_hash: &[u8],
) -> Result<ConflictingTranscationInfo, diesel::result::Error> {
    let (block, transactions, conflicting_transactions_info) =
        single_block_with_conflicting_transactions_data(conn, block_hash)?;

    let txid_to_tx_map: HashMap<Vec<u8>, &Transaction> = transactions
        .iter()
        .map(|tx| (tx.txid.clone(), tx))
        .collect();

    let block_with_conflicting_transctions = ConflictingTranscationInfo {
        conflicting_transaction_sets: conflicting_transactions_info
            .iter()
            .map(|ct| ConflictingTransactionSet {
                template_transactions: ct
                    .template_txids
                    .iter()
                    .map(|txid| (*txid_to_tx_map.get(txid).unwrap()).clone())
                    .collect(),
                block_transactions: ct
                    .block_txids
                    .iter()
                    .map(|txid| (*txid_to_tx_map.get(txid).unwrap()).clone())
                    .collect(),
                conflicting_outpoints: ct
                    .conflicting_outpoints_txids
                    .iter()
                    .zip(&ct.conflicting_outpoints_vouts)
                    .map(|(txid, vout)| ConflictingOutpoint {
                        txid: txid.clone(),
                        vout: *vout,
                    })
                    .collect(),
            })
            .collect(),
        block,
    };

    Ok(block_with_conflicting_transctions)
}

const QUERY_DEBUG_SANCTIONED_TABLE_TOTAL: &str = r#"
SELECT 
    pool_name,
    COUNT(CASE WHEN in_block = TRUE AND in_template = TRUE THEN 1 END) AS in_both,
    COUNT(CASE WHEN in_block = FALSE AND in_template = TRUE THEN 1 END) AS only_in_template,
    COUNT(CASE WHEN in_block = TRUE AND in_template = FALSE THEN 1 END) AS only_in_block
FROM sanctioned_transaction_info
JOIN 
    block on block.id = sanctioned_transaction_info.block_id
GROUP BY
    pool_name
ORDER BY pool_name ASC;
;"#;

pub fn debug_sanctioned_table(
    conn: &mut PgConnection,
) -> Result<Vec<PoolSanctionedTableEntry>, diesel::result::Error> {
    sql_query(QUERY_DEBUG_SANCTIONED_TABLE_TOTAL).load::<PoolSanctionedTableEntry>(conn)
}

pub fn missing_sanctioned_txns_for_block(
    req_hash: &[u8],
    conn: &mut PgConnection,
) -> Result<Vec<MissingSanctionedTransaction>, diesel::result::Error> {
    let this_block_id: i32;
    {
        use schema::block::dsl::*;
        this_block_id = block.select(id).filter(hash.eq(req_hash)).first(conn)?;
    }

    let transactions_and_sanction_info: Vec<MissingSanctionedTransaction>;
    {
        use schema::sanctioned_transaction_info::dsl::*;
        //use schema::transaction_only_in_template::dsl::*;
        use schema::transaction::dsl::*;

        transactions_and_sanction_info = transaction
            .inner_join(sanctioned_transaction_info)
            .inner_join(schema::transaction_only_in_template::table)
            .filter(
                block_id
                    .eq(this_block_id)
                    .and(in_block.eq(false))
                    .and(in_template.eq(true)),
            )
            .load::<(
                Transaction,
                SanctionedTransactionInfo,
                TransactionOnlyInTemplate,
            )>(conn)?
            .iter()
            .map(
                |(tx, info, tx_not_in_template_info)| MissingSanctionedTransaction {
                    transaction: tx.clone(),
                    missing_info: tx_not_in_template_info.clone(),
                    addresses: info.addresses.clone(),
                },
            )
            .collect::<Vec<MissingSanctionedTransaction>>()
    }

    Ok(transactions_and_sanction_info)
}

const QUERY_AVG_POOL_FEES: &str = r#"
SELECT 
    pool_name,
    count(pool_name),
    percentile_disc(0.5) within group (order by (block_cb_fees::float / template_cb_fees::float)) median,
    percentile_disc(0.25) within group (order by (block_cb_fees::float / template_cb_fees::float)) q1,
    percentile_disc(0.75) within group (order by (block_cb_fees::float / template_cb_fees::float)) q3
FROM block
GROUP BY
    pool_name
HAVING 
    count(pool_name) > 10
ORDER BY median DESC
;"#;

pub fn avg_fees_by_pool(
    conn: &mut PgConnection,
) -> Result<Vec<AvgPoolFees>, diesel::result::Error> {
    sql_query(QUERY_AVG_POOL_FEES).load::<AvgPoolFees>(conn)
}

fn transaction_only_in_template_by_block_id(
    p_block_id: i32,
    conn: &mut PgConnection,
) -> Result<Vec<(TransactionOnlyInTemplate, Transaction)>, diesel::result::Error> {
    use schema::transaction_only_in_template::dsl::*;
    transaction_only_in_template
        .filter(block_id.eq(p_block_id))
        .inner_join(schema::transaction::table)
        .order_by(position.asc())
        .load(conn)
}

fn transaction_only_in_block_by_block_id(
    p_block_id: i32,
    conn: &mut PgConnection,
) -> Result<Vec<(TransactionOnlyInBlock, Transaction)>, diesel::result::Error> {
    use schema::transaction_only_in_block::dsl::*;
    transaction_only_in_block
        .filter(block_id.eq(p_block_id))
        .inner_join(schema::transaction::table)
        .order_by(position.asc())
        .load(conn)
}

const QUERY_MISSING_TRANSACTIONS: &str = r#"
    SELECT
        count(*) cnt,
        max(block_id) max_block_id,
        transaction_txid txid
    FROM transaction_only_in_template
    JOIN transaction
        ON transaction.txid = transaction_txid
    JOIN block
	    on block.id = block_id
    WHERE
	    transaction_only_in_template.position < (block.template_tx - block.template_tx * 0.02)
	    AND
	    block.block_tx > 1
    GROUP BY transaction_txid
    HAVING
        count(*) > 2
    ORDER BY
        max_block_id
        DESC
    LIMIT $1
    OFFSET $2;"#;

const QUERY_COUNT_MISSING_TRANSACTIONS: &str = r#"
    SELECT
        COUNT(*)
    FROM
        (
            SELECT DISTINCT
                transaction_txid
            FROM
                transaction_only_in_template
            JOIN block
                on block.id = block_id
            WHERE
                transaction_only_in_template.position < (block.template_tx - block.template_tx * 0.02)
                AND
                block.block_tx > 1
            GROUP BY transaction_txid
            HAVING
                COUNT(transaction_txid) > 2
        ) AS tx_missing_from_multiple_blocks;
    ;"#;

#[derive(QueryableByName)]
struct MissingTransactionCountInfo {
    #[diesel(sql_type = Bytea)]
    txid: Vec<u8>,
}

#[derive(Debug, QueryableByName)]
struct MissingTransactionCount {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

pub fn missing_transactions(
    conn: &mut PgConnection,
    page: u32,
) -> Result<(Vec<MissingTransaction>, u32), diesel::result::Error> {
    let missing_transactions_data = missing_transactions_data(conn, page)?;

    let block_id_to_block_map: HashMap<i32, &Block> = missing_transactions_data
        .blocks
        .iter()
        .map(|b| (b.id, b))
        .collect::<HashMap<i32, &Block>>();

    let mut missing_txns: Vec<MissingTransaction> = vec![];
    for tx in missing_transactions_data.transactions {
        let only_in_these_templates = missing_transactions_data
            .txns_only_in_template
            .iter()
            .filter(|tx_only_in_template| tx_only_in_template.transaction_txid == tx.txid);

        let mut blocks_the_transaction_is_missing_from: Vec<MissingTransactionBlockInfo> =
            only_in_these_templates
                .map(|oitt| {
                    let block = block_id_to_block_map.get(&oitt.block_id).unwrap();
                    MissingTransactionBlockInfo {
                        hash: block.hash.clone(),
                        height: block.height,
                        template_position: oitt.position,
                        time: block.block_time,
                        mempool_age: oitt.mempool_age_seconds,
                        pool: block.pool_name.clone(),
                        last_block_pkg_feerate: *block.block_pkg_feerates.last().unwrap_or(&0.0),
                        template_tx_count: block.template_tx,
                    }
                })
                .collect();

        blocks_the_transaction_is_missing_from.sort_by_key(|b| b.height);
        blocks_the_transaction_is_missing_from.reverse();

        missing_txns.push(MissingTransaction {
            transaction: tx.clone(),
            blocks: blocks_the_transaction_is_missing_from,
        });
    }

    missing_txns.sort_by_key(|b| std::cmp::Reverse(b.max_height()));

    let max_pages: u32 = (missing_transactions_data.total_missing_count as f32
        / MAX_BLOCKS_PER_PAGE as f32)
        .ceil() as u32;
    Ok((missing_txns, max_pages))
}

struct MissingTransactionsData {
    transactions: Vec<Transaction>,
    txns_only_in_template: Vec<TransactionOnlyInTemplate>,
    blocks: Vec<Block>,
    total_missing_count: i64,
}

fn missing_transactions_data(
    conn: &mut PgConnection,
    page: u32,
) -> Result<MissingTransactionsData, diesel::result::Error> {
    use schema::block::dsl::*;
    use schema::transaction::dsl::*;
    use schema::transaction_only_in_template::dsl::*;

    let missing_counts: Vec<MissingTransactionCount> =
        sql_query(QUERY_COUNT_MISSING_TRANSACTIONS).load(conn)?;
    let total_missing_count = missing_counts.first().unwrap();

    let count_info: Vec<MissingTransactionCountInfo> = sql_query(QUERY_MISSING_TRANSACTIONS)
        .bind::<BigInt, _>(MAX_BLOCKS_PER_PAGE)
        .bind::<BigInt, _>(page as i64 * MAX_BLOCKS_PER_PAGE)
        .load(conn)?;
    let txids: Vec<Vec<u8>> = count_info.iter().map(|ci| ci.txid.clone()).collect();
    let transactions: Vec<Transaction> =
        transaction.filter(txid.eq_any(txids.clone())).load(conn)?;

    let txns_only_in_template: Vec<TransactionOnlyInTemplate> = transaction_only_in_template
        .distinct()
        .filter(transaction_txid.eq_any(txids))
        .order_by(block_id.desc())
        .load::<TransactionOnlyInTemplate>(conn)?;

    let block_ids: Vec<i32> = txns_only_in_template.iter().map(|a| a.block_id).collect();

    let blocks: Vec<Block> = block.filter(id.eq_any(block_ids)).load::<Block>(conn)?;

    Ok(MissingTransactionsData {
        transactions,
        txns_only_in_template,
        blocks,
        total_missing_count: total_missing_count.count,
    })
}

pub fn single_missing_transaction_data(
    req_txid: &[u8],
    conn: &mut PgConnection,
) -> Result<(Transaction, Vec<TransactionOnlyInTemplate>, Vec<Block>), diesel::result::Error> {
    use schema::block::dsl::*;
    use schema::transaction::dsl::*;
    use schema::transaction_only_in_template::dsl::*;

    let tx: Transaction = transaction.filter(txid.eq(req_txid)).first(conn)?;

    let transaction_only_in_templates_not_included_in_block: Vec<TransactionOnlyInTemplate> =
        transaction_only_in_template
            .distinct()
            .filter(transaction_txid.eq(req_txid))
            .order_by(block_id.desc())
            .load::<TransactionOnlyInTemplate>(conn)?;

    let block_ids: Vec<i32> = transaction_only_in_templates_not_included_in_block
        .iter()
        .map(|a| a.block_id)
        .collect();

    let blocks: Vec<Block> = block.filter(id.eq_any(block_ids)).load::<Block>(conn)?;

    Ok((
        tx,
        transaction_only_in_templates_not_included_in_block,
        blocks,
    ))
}

pub fn single_missing_transaction(
    req_txid: &[u8],
    conn: &mut PgConnection,
) -> Result<MissingTransaction, diesel::result::Error> {
    let (tx, transaction_only_in_templates_not_included_in_block, block_infos) =
        single_missing_transaction_data(req_txid, conn)?;

    let block_id_to_block_map: HashMap<i32, &Block> = block_infos
        .iter()
        .map(|b| (b.id, b))
        .collect::<HashMap<i32, &Block>>();

    let mut blocks_the_transaction_is_missing_from: Vec<MissingTransactionBlockInfo> =
        transaction_only_in_templates_not_included_in_block
            .iter()
            .map(|oitt| {
                let block = block_id_to_block_map.get(&oitt.block_id).unwrap();
                MissingTransactionBlockInfo {
                    hash: block.hash.clone(),
                    height: block.height,
                    template_position: oitt.position,
                    mempool_age: oitt.mempool_age_seconds,
                    time: block.block_time,
                    pool: block.pool_name.clone(),
                    last_block_pkg_feerate: *block.block_pkg_feerates.last().unwrap_or(&0.0),
                    template_tx_count: block.template_tx,
                }
            })
            .collect();

    blocks_the_transaction_is_missing_from.sort_by_key(|b| b.height);
    blocks_the_transaction_is_missing_from.reverse();

    let missing_tx = MissingTransaction {
        transaction: tx,
        blocks: blocks_the_transaction_is_missing_from,
    };

    Ok(missing_tx)
}

pub fn get_recent_sanctioned_utxo_scan_info(
    conn: &mut PgConnection,
) -> Result<SanctionedUtxoScanInfo, diesel::result::Error> {
    use schema::sanctioned_utxo_scan_info::dsl::*;
    sanctioned_utxo_scan_info
        .order_by(end_time.desc())
        .limit(1)
        .first(conn)
}

pub fn sanctioned_utxo_scan_infos(
    conn: &mut PgConnection,
    page: u32,
) -> Result<(Vec<SanctionedUtxoScanInfo>, u32), diesel::result::Error> {
    use schema::sanctioned_utxo_scan_info::dsl::*;
    let sanctioned_utxo_scan_infos: Vec<SanctionedUtxoScanInfo> = sanctioned_utxo_scan_info
        .limit(MAX_UTXOSET_SCANS_PER_PAGE)
        .offset(MAX_UTXOSET_SCANS_PER_PAGE * page as i64)
        .order(end_time.desc())
        .load::<SanctionedUtxoScanInfo>(conn)?;

    let scan_count: i64 = *sanctioned_utxo_scan_info
        .select(count(end_time))
        .load(conn)?
        .first()
        .unwrap();
    let max_pages: u32 = (scan_count as f32 / MAX_UTXOSET_SCANS_PER_PAGE as f32).ceil() as u32;
    Ok((sanctioned_utxo_scan_infos, max_pages))
}

pub fn debug_template_selection_infos(
    conn: &mut PgConnection,
    page: u32,
) -> Result<(Vec<DebugTemplateSelectionInfosAndBlock>, u32), diesel::result::Error> {
    use schema::debug_template_selection::dsl::*;

    let block_ids: Vec<i32> = debug_template_selection
        .select(block_id)
        .limit(MAX_BLOCKS_PER_PAGE)
        .distinct()
        .offset(MAX_BLOCKS_PER_PAGE * page as i64)
        .order(block_id.desc())
        .load::<i32>(conn)?;

    let infos: Vec<DebugTemplateSelectionInfo> = debug_template_selection
        .filter(block_id.eq_any(block_ids.clone()))
        .order(template_time.desc())
        .load::<DebugTemplateSelectionInfo>(conn)?;

    let blocks: Vec<Block>;
    {
        use schema::block::dsl::*;
        blocks = block
            .filter(id.eq_any(block_ids))
            .order(block_time.desc())
            .load::<Block>(conn)?;
    }

    let mut infos_and_blocks: Vec<DebugTemplateSelectionInfosAndBlock> = vec![];
    for b in blocks {
        infos_and_blocks.push(DebugTemplateSelectionInfosAndBlock {
            infos: infos
                .iter()
                .filter(|info| info.block_id == b.id)
                .cloned()
                .collect::<Vec<DebugTemplateSelectionInfo>>(),
            block: b,
        });
    }

    let infos_count: i64 = *debug_template_selection
        .select(sql::<BigInt>("COUNT( DISTINCT block_id )"))
        .load(conn)?
        .first()
        .unwrap();
    let max_pages: u32 = (infos_count as f32 / MAX_BLOCKS_PER_PAGE as f32).ceil() as u32;
    Ok((infos_and_blocks, max_pages))
}

pub fn blocks_with_missing_sanctioned(
    conn: &mut PgConnection,
) -> Result<Vec<Block>, diesel::result::Error> {
    use schema::block::dsl::*;
    let blocks = block
        .filter(sanctioned_missing_tx.gt(0))
        .order(block_time.desc())
        .load::<Block>(conn)?;
    Ok(blocks)
}

pub fn debug_templates_and_blocks_with_sanctioned_tx(
    conn: &mut PgConnection,
) -> Result<Vec<Block>, diesel::result::Error> {
    use schema::block::dsl::*;
    let blocks = block
        .filter(template_sanctioned.gt(0).or(block_sanctioned.gt(0)))
        .order(block_time.desc())
        .load::<Block>(conn)?;
    Ok(blocks)
}

pub fn get_node_info(conn: &mut PgConnection) -> Result<String, diesel::result::Error> {
    use schema::node_info::dsl::*;
    let info = node_info.select(version).first::<String>(conn)?;
    Ok(info)
}
