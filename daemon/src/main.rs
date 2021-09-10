use std::collections::{HashMap, HashSet, VecDeque};
use std::thread;
use std::time;
use std::time::Instant;

use bitcoin::hash_types::Txid;
use bitcoin::hashes::Hash;
use bitcoin::{Amount, Block};
use bitcoin_pool_identification::PoolIdentification;
use simple_logger::SimpleLogger;

use miningpool_observer_shared::bitcoincore_rpc::json::{
    GetBlockTemplateModes, GetBlockTemplateResult, GetBlockTemplateRules, GetBlockTxFeesResult,
    ScanTxOutRequest,
};
use miningpool_observer_shared::bitcoincore_rpc::{Client, Error, RpcApi};
use miningpool_observer_shared::{
    config, db_pool, model as shared_model, prometheus_metric_server,
};

#[macro_use]
extern crate diesel_migrations;

mod db;
mod metrics;
mod model;
mod processing;

const WAIT_TIME_BETWEEN_TEMPLATE_QUERIES: time::Duration = time::Duration::from_secs(10);
const WAIT_TIME_BETWEEN_CONNPOOL_GETCONNECTION: time::Duration = time::Duration::from_secs(1);
const WAIT_TIME_BETWEEN_UTXO_SET_SCANS: time::Duration =
    std::time::Duration::from_secs(60 * 60 * 3); // 3 hours
const WAIT_TIME_BETWEEN_FAILED_UTXO_SET_SCANS: time::Duration =
    std::time::Duration::from_secs(60 * 5); // 5 minutes
const MAX_OLD_TEMPLATES: usize = 15;

const LOG_TARGET_RPC: &str = "rpc";
const LOG_TARGET_REIDUNKNOWNPOOLS: &str = "re-id_unknown_pools";
const LOG_TARGET_UTXOSETSCAN: &str = "utxo_set_scan";
const LOG_TARGET_STATS: &str = "stats";
const LOG_TARGET_DBPOOL: &str = "dbpool";
const LOG_TARGET_STARTUP: &str = "startup";

fn main() {
    let config = match config::load_daemon_config() {
        Ok(config) => config,
        Err(e) => panic!("Could not load the configuration: {}", e),
    };

    metrics::RUNTIME_START_TIME.set(
        time::SystemTime::now()
            .duration_since(time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
    );

    match SimpleLogger::new().with_level(config.log_level).init() {
        Ok(_) => (),
        Err(e) => panic!("Could not setup logger: {}", e),
    }

    let rpc_client = match Client::new(config.rpc_url.clone(), config.rpc_auth.clone()) {
        Ok(config) => config,
        Err(e) => panic!("Could not setup the Bitcoin Core RPC client: {}", e),
    };
    match rpc_client.get_blockchain_info() {
        Ok(blockchaininfo) => {
            log::info!(target: LOG_TARGET_STARTUP, "Successfully connected to the Bitcoin Core RPC server: chain={}, blocks={}, verification progress={}.", blockchaininfo.chain, blockchaininfo.blocks, blockchaininfo.verification_progress);
        }
        Err(e) => {
            log::error!(target: LOG_TARGET_STARTUP, "Could not connect to the Bitcoin Core RPC server. Is the user/password correct?: {}", e);
            panic!("Could not connect to the Bitcoin Core RPC server.")
        }
    }

    let conn_pool = match db_pool::new(&config.database_url) {
        Ok(pool) => pool,
        Err(e) => panic!("Could not create a Postgres connection pool: {}", e),
    };
    log::info!(
        target: LOG_TARGET_STARTUP,
        "Succesfully created a database connection pool with a max size of {} connections.",
        conn_pool.max_size()
    );

    if config.prometheus.enable {
        if let Err(e) = prometheus_metric_server::start_http_server(&config.prometheus.address) {
            log::error!(
                target: LOG_TARGET_STARTUP,
                "Could not start the Prometheus Metrics Server on {}: {}",
                config.prometheus.address,
                e
            );
            panic!("Could not start the Prometheus Metrics Server.")
        }
    } else {
        log::info!(
            target: LOG_TARGET_STARTUP,
            "Not starting the Prometheus Metric Server as it is disabled via the config file."
        )
    }

    startup_db_mirgation(&conn_pool);

    // Sanctioned UTXO (re)scan
    let utxo_set_scan_conn_pool = match db_pool::new(&config.database_url) {
        Ok(pool) => pool,
        Err(e) => panic!(
            "During startup: Could not create a Postgres connection pool: {}",
            e
        ),
    };
    let rpc_client_utxo_set_scan =
        match Client::new(config.rpc_url.clone(), config.rpc_auth.clone()) {
            Ok(config) => config,
            Err(e) => panic!(
                "During startup: Could not setup the Bitcoin Core RPC client: {}",
                e
            ),
        };
    start_sanctioned_utxos_scan_thread(rpc_client_utxo_set_scan, utxo_set_scan_conn_pool);

    // Retry pool identification for "Unkown" pools
    let reid_conn_pool = match db_pool::new(&config.database_url) {
        Ok(pool) => pool,
        Err(e) => panic!(
            "During startup: Could not create a Postgres connection pool: {}",
            e
        ),
    };
    let reid_rpc_client = match Client::new(config.rpc_url.clone(), config.rpc_auth) {
        Ok(config) => config,
        Err(e) => panic!(
            "During startup: Could not setup the Bitcoin Core RPC client: {}",
            e
        ),
    };
    start_retry_unknown_pool_identification_thread(reid_rpc_client, reid_conn_pool);

    match rpc_client.get_network_info() {
        Ok(network_info) => {
            let version = network_info.subversion.replace("/", "").replace(":", " ");
            log::info!(
                target: LOG_TARGET_STARTUP,
                "Block templates are generated by Bitcoin Core with version {}.",
                version
            );
            match conn_pool.get() {
                Ok(conn) => {
                    if let Err(e) = db::update_node_info(&version, &conn) {
                        log::error!(
                            target: LOG_TARGET_STARTUP,
                            "Could not update the node information in the database: {}",
                            e
                        );
                        panic!("During startup: Could not update the node information in the database.");
                    }
                }
                Err(e) => {
                    log::error!(
                        target: LOG_TARGET_STARTUP,
                        "Could not get a connection from the connection pool: {}",
                        e
                    );
                    panic!("During startup: Could not get a database connection from the connection pool.");
                }
            }
        }
        Err(e) => {
            log::error!(target: LOG_TARGET_STARTUP, "Could not connect to the Bitcoin Core RPC server. Is the user/password correct?: {}", e);
            panic!("Could not connect to the Bitcoin Core RPC server.")
        }
    }

    main_loop(&rpc_client, &conn_pool);
}

fn startup_db_mirgation(conn_pool: &db_pool::PgPool) {
    match conn_pool.get() {
        Ok(conn) => {
            match db::run_migrations(&conn) {
                Ok(_) => log::info!(
                    target: LOG_TARGET_STARTUP,
                    "Database migrations successful."
                ),
                Err(e) => {
                    log::error!(
                        target: LOG_TARGET_STARTUP,
                        "Could not run the PostgreSQL database migration: {}",
                        e
                    );
                    panic!("During startup: Could not run the PostgreSQL database migration.");
                }
            };
        }
        Err(e) => {
            log::error!(
                target: LOG_TARGET_STARTUP,
                "Could not get a connection from the connection pool: {}",
                e
            );
            panic!("During startup: Could not get a database connection from the connection pool.");
        }
    }
}

fn main_loop(rpc: &Client, db_pool: &db_pool::PgPool) {
    // stores up to the last MAX_OLD_TEMPLATES GetBlockTemplateResults to lookup older templates
    // based on miner block timestamps.
    let mut last_templates: VecDeque<GetBlockTemplateResult> =
        VecDeque::with_capacity(MAX_OLD_TEMPLATES);

    loop {
        metrics::RUNTIME_TEMPLATES_IN_MEMORY.set(last_templates.len() as i64);

        let current_template = match rpc.get_block_template(
            GetBlockTemplateModes::Template,
            &[
                GetBlockTemplateRules::SegWit,
                GetBlockTemplateRules::Taproot,
            ],
            &[],
        ) {
            Ok(t) => t,
            Err(e) => {
                log::error!(
                    target: LOG_TARGET_RPC,
                    "Could not get a block template from the Bitcoin Core RPC server: {}",
                    e
                );
                metrics::ERROR_RPC.inc();
                thread::sleep(WAIT_TIME_BETWEEN_TEMPLATE_QUERIES);
                continue;
            }
        };
        metrics::RUNTIME_REQUESTED_TEMPLATES.inc();

        log_template_infos(&current_template);

        // if we don't have any templates yet, just add the current_template
        // and finish the loop early.
        if last_templates.is_empty() {
            last_templates.push_back(current_template);
            thread::sleep(WAIT_TIME_BETWEEN_TEMPLATE_QUERIES);
            continue;
        }

        let previous_template = match last_templates.make_contiguous().last() {
            Some(t) => t,
            None => {
                processing::log_processing_error(&format!(
                    "Could not get the previous_template from last_templates (size: {}).",
                    last_templates.len()
                ));
                metrics::ERROR_PROCESSING.inc(); // TODO: this can be remove as log_processing_error already increases it
                panic!("last_templates should not be empty.");
            }
        };

        // To detect if a block has been mined, we check the if the previous block hashes
        // of the previous_template and the current_template are different. If they are
        // the same, no block has been mined and we wait before getting a new template.
        if previous_template.previous_block_hash == current_template.previous_block_hash {
            last_templates.push_back(current_template);
            if last_templates.len() > MAX_OLD_TEMPLATES {
                last_templates.pop_front();
            }
            thread::sleep(WAIT_TIME_BETWEEN_TEMPLATE_QUERIES);
            continue;
        }

        // The previous block hash of the previous_template and the current_template are
        // different. This means the chain tip changed. We request the block with the 
        // current_templates previous block hash. In most cases, this should be the block
        // that was just mined.
        let bitcoin_block = match rpc.get_block(&current_template.previous_block_hash) {
            Ok(b) => {
                metrics::RUNTIME_REQUESTED_BLOCKS.inc();
                b
            }
            Err(e) => {
                log::error!(
                    target: LOG_TARGET_RPC,
                    "Could not get the block with the hash {} from the Bitcoin Core RPC server: {}",
                    current_template.previous_block_hash,
                    e
                );
                log::error!(
                    target: LOG_TARGET_RPC,
                    "Skipping the processing of block {}.",
                    current_template.previous_block_hash
                );
                metrics::ERROR_RPC.inc();
                thread::sleep(WAIT_TIME_BETWEEN_TEMPLATE_QUERIES);
                continue;
            }
        };

        // When something changed, we can't be sure if the templates in last_templates
        // are actually templates for the block we just requested. For example, two block
        // might have arrived at our node in rapid succession and we didn't query for a
        // template in between.
        // To check that we can compare the block with one of our last_templates, we
        // compare the previous block hashes of block and the previous_template.
        // Only if the hashes are equal, we can compare the template and block.
        // If they are not equal, we might have skipped one or more blocks.
        if bitcoin_block.header.prev_blockhash != previous_template.previous_block_hash {
            log::warn!(
                target: processing::LOG_TARGET_PROCESSING,
                "Can't compare the previous_template to the new block. Was there a reorg or multiple blocks found in rapid succession?",
            );
            metrics::RUNTIME_SKIPPED_BLOCK_EVENTS.inc();
            
            // We can however still compare the previous_template to it's respective block.
            let hash_of_missed_block = match rpc.get_block_hash(previous_template.height) {
                Ok(hash) => hash,
                Err(e) => {
                    log::error!(
                        target: LOG_TARGET_RPC,
                        "Could not get the block hash of block {} from the Bitcoin Core RPC server: {}",
                        previous_template.height,
                        e
                    );
                    thread::sleep(WAIT_TIME_BETWEEN_TEMPLATE_QUERIES);
                    continue;
                }
            };

            let bitcoin_block = match rpc.get_block(&hash_of_missed_block) {
                Ok(b) => {
                    metrics::RUNTIME_REQUESTED_BLOCKS.inc();
                    b
                }
                Err(e) => {
                    log::error!(
                        target: LOG_TARGET_RPC,
                        "Could not get the block with the hash {} from the Bitcoin Core RPC server: {}",
                        current_template.previous_block_hash,
                        e
                    );
                    log::error!(
                        target: LOG_TARGET_RPC,
                        "Skipping the processing of block missed block {}.",
                        current_template.previous_block_hash
                    );
                    thread::sleep(WAIT_TIME_BETWEEN_TEMPLATE_QUERIES);
                    continue;
                }
            };

            // TODO: Once Bitcoin Core v22 with getblock verbosity level 2 is released
            // and rust-bitcoincore-rpc has can get the block and fees in on RPC call,
            // then the above getblock call and this can be merged.
            let block_tx_fees = match rpc.get_block_txid_fee(&hash_of_missed_block) {
                Ok(b) => b,
                Err(e) => {
                    log::error!(target: LOG_TARGET_RPC, "Could not get the txids and fees for block with the hash {} from the Bitcoin Core RPC server: {}", current_template.previous_block_hash, e);
                    log::error!(target: processing::LOG_TARGET_PROCESSING, "Skipping the processing of block {}.", current_template.previous_block_hash);
                    metrics::ERROR_RPC.inc();
                    thread::sleep(WAIT_TIME_BETWEEN_TEMPLATE_QUERIES);
                    continue;
                }
            };

            log::info!(
                target: LOG_TARGET_STATS,
                "Processing missed block {} mined by {}",
                bitcoin_block.block_hash(),
                match bitcoin_block.identify_pool() {
                    Some(pool) => pool.name,
                    None => "UNKNOWN".to_string(),
                }
            );

            process(
                &rpc,
                &db_pool,
                &bitcoin_block,
                &block_tx_fees,
                &mut last_templates,
            );            

            last_templates.push_back(current_template);
            continue;
        }

        // TODO: Once Bitcoin Core v22 with getblock verbosity level 2 is released
        // and rust-bitcoincore-rpc has can get the block and fees in on RPC call,
        // then the above getblock call and this can be merged.
        let block_tx_fees = match rpc.get_block_txid_fee(&current_template.previous_block_hash) {
            Ok(b) => b,
            Err(e) => {
                log::error!(target: LOG_TARGET_RPC, "Could not get the txids and fees for block with the hash {} from the Bitcoin Core RPC server: {}", current_template.previous_block_hash, e);
                log::error!(target: processing::LOG_TARGET_PROCESSING, "Skipping the processing of block {}.", current_template.previous_block_hash);
                metrics::ERROR_RPC.inc();
                thread::sleep(WAIT_TIME_BETWEEN_TEMPLATE_QUERIES);
                continue;
            }
        };

        log::info!(
            target: LOG_TARGET_STATS,
            "New block detected {} mined by {}",
            bitcoin_block.block_hash(),
            match bitcoin_block.identify_pool() {
                Some(pool) => pool.name,
                None => "UNKNOWN".to_string(),
            }
        );

        process(
            &rpc,
            &db_pool,
            &bitcoin_block,
            &block_tx_fees,
            &mut last_templates,
        );
    }
}

fn process(
    rpc: &Client,
    db_pool: &db_pool::PgPool,
    bitcoin_block: &Block,
    block_tx_fees: &GetBlockTxFeesResult,
    last_templates: &mut VecDeque<GetBlockTemplateResult>,
){
    let block_tx_data = processing::build_block_tx_data(&bitcoin_block, &block_tx_fees);

    // For best possible comparison we want to compare a template and a block
    // where our template was generated at the same time as the pool generated
    // his template. As we can't know when exactly the pool generated his
    // template, we assume that it must have been before or at the
    // timestamp included in the block headder. During mining, the miners likely
    // engages in nTime rolling, which increases the timestamp in the block header.
    // Additionally, we don't know if the pools clock is accurate.
    //
    // Thus, to aid template selection, we pick the template sharing the most transactions
    // with the block out of the templates generated before the timestamp in the block header.
    let template = select_best_template_for_block(&last_templates, block_tx_data.txids.clone());

    let template_tx_data = processing::build_template_tx_data(template);

    let template_tx_packages = processing::build_packages(&template_tx_data.txinfos);
    let template_pkg_feerates: Vec<f32> =
        template_tx_packages.iter().map(|p| p.feerate()).collect();
    let template_pkg_weights: Vec<i64> = template_tx_packages
        .iter()
        .map(|p| p.weight() as i64)
        .collect();

    let block_tx_packages = processing::build_packages(&block_tx_data.txinfos);
    let block_pkg_feerates: Vec<f32> = block_tx_packages.iter().map(|p| p.feerate()).collect();
    let block_pkg_weights: Vec<i64> = block_tx_packages
        .iter()
        .map(|p| p.weight() as i64)
        .collect();

    // Set diffs to get the transactions that are in the block
    // but not in the template and the other way around.
    let txids_only_in_block: HashSet<&Txid> = block_tx_data
        .txids
        .difference(&template_tx_data.txids)
        .collect();
    let txids_only_in_template: HashSet<&Txid> = template_tx_data
        .txids
        .difference(&block_tx_data.txids)
        .collect();
    let shared_tx: usize = block_tx_data
        .txids
        .intersection(&template_tx_data.txids)
        .count();

    log::info!(
        target: LOG_TARGET_STATS,
        "Transactions in block: shared={}, missing={}, extra={}",
        shared_tx,
        txids_only_in_template.len(),
        txids_only_in_block.len()
    );

    let connection = match db_pool.get() {
        Ok(c) => c,
        Err(e) => {
            log::error!(
                target: LOG_TARGET_DBPOOL,
                "Could not get a connection from the connection pool. Retrying in {:?}. Error: {}",
                WAIT_TIME_BETWEEN_CONNPOOL_GETCONNECTION,
                e
            );
            metrics::ERROR_DBPOOL.inc();
            thread::sleep(WAIT_TIME_BETWEEN_CONNPOOL_GETCONNECTION);
            match db_pool.get() {
                Ok(c) => c,
                Err(e) => {
                    log::error!(target: LOG_TARGET_DBPOOL, "Could not get a connection from the connection pool. Skipping processing of block {}. Error: {}", bitcoin_block.block_hash(), e);
                    metrics::ERROR_DBPOOL.inc();
                    return;
                }
            }
        }
    };

    let sanctioned_utxos = match db::get_sanctioned_utxos(&connection) {
        Ok(utxos) => utxos,
        Err(e) => {
            processing::log_processing_error(&format!("Could not load the sanctioned utxos from the database. Using empty UTXO set. Error: {}", e));
            vec![]
        }
    };
    let outpoint_to_sanctioned_utxo_map =
        processing::build_outpoint_to_sanctioned_utxo_map(&sanctioned_utxos);

    let sanctioned_missing_tx = processing::get_sanctioned_missing_tx_count(
        &txids_only_in_template,
        &template_tx_data,
        &outpoint_to_sanctioned_utxo_map,
    );

    let block_fees: Amount =
        Amount::from_sat(block_tx_fees.tx.iter().map(|tx| tx.fee.as_sat()).sum());
    let template_fees: Amount =
        Amount::from_sat(template.transactions.iter().map(|tx| tx.fee.as_sat()).sum());
    let missing_tx: i32 = txids_only_in_template.len() as i32;
    let extra_tx: i32 = txids_only_in_block.len() as i32;
    let block = processing::build_block(
        &bitcoin_block,
        &template,
        &template_tx_data.txid_to_txinfo_map,
        &template_pkg_weights,
        &template_pkg_feerates,
        &block_pkg_weights,
        &block_pkg_feerates,
        missing_tx,
        sanctioned_missing_tx as i32,
        extra_tx,
        shared_tx as i32,
        &block_fees,
        &template_fees,
        &outpoint_to_sanctioned_utxo_map,
    );

    let block_id = match db::insert_block(&block, &connection) {
        Ok(id) => id,
        Err(e) => {
            processing::log_processing_error(&format!("Could not insert the block into the database. Skipping the remaining processing. Error: {}", e));
            return;
        }
    };

    let mut transactions: HashMap<Vec<u8>, shared_model::Transaction> = HashMap::new();

    let conflicting_transactions = processing::build_conflicting_transactions(
        block_id,
        &txids_only_in_template,
        &template_tx_data.txid_to_txinfo_map,
        &txids_only_in_block,
        &block_tx_data.txid_to_txinfo_map,
        &mut transactions,
        &outpoint_to_sanctioned_utxo_map,
    );

    if !conflicting_transactions.is_empty() {
        log::info!(
            target: LOG_TARGET_STATS,
            "Between the template and the block are {} sets of conflicting transactions.",
            conflicting_transactions.len()
        );
        metrics::STAT_CONFLICTING_TRANSACTION_SETS.inc_by(conflicting_transactions.len() as u64);
    }

    let template_txid_to_mempool_age = mempool_age_seconds(rpc, &txids_only_in_template);
    let transactions_only_in_template = processing::build_transactions_only_in_template(
        block_id,
        &txids_only_in_template,
        &template_tx_data.txid_to_txinfo_map,
        &template_txid_to_mempool_age,
        &mut transactions,
        &outpoint_to_sanctioned_utxo_map,
    );
    let transactions_only_in_block = processing::build_transactions_only_in_block(
        block_id,
        &txids_only_in_block,
        &block_tx_data.txid_to_txinfo_map,
        &mut transactions,
        &outpoint_to_sanctioned_utxo_map,
    );
    let sanctioned_transaction_infos = processing::build_sanctioned_transaction_infos(
        block_id,
        &block_tx_data,
        &template_tx_data.txids,
        &template_tx_data.txid_to_txinfo_map,
        &txids_only_in_block,
        &outpoint_to_sanctioned_utxo_map,
        &mut transactions,
    );
    if !sanctioned_transaction_infos.is_empty() {
        log::info!(
            target: LOG_TARGET_STATS,
            "There are a total of {} sanctioned transactions in this template and block.",
            sanctioned_transaction_infos.len()
        );
        metrics::STAT_SANCTIONED_TRANSACTIONS.inc_by(sanctioned_transaction_infos.len() as u64);
    }

    let debug_template_selection_infos = processing::build_debug_template_selection_infos(
        block_id,
        &last_templates,
        block_tx_data.txids,
        template.current_time,
    );

    if let Err(e) = db::insert_transactions(transactions.values().cloned().collect(), &connection) {
        processing::log_processing_error(&format!("Could not insert the transactions into the database. Skipping the remaining processing. Unclean database state! Error: {}", e));
        return;
    }
    if let Err(e) = db::insert_transactions_only_in_block(transactions_only_in_block, &connection) {
        processing::log_processing_error(&format!("Could not insert the transactions_only_in_block into the database. Skipping the remaining processing. Unclean database state! Error: {}", e));
        return;
    }
    if let Err(e) =
        db::insert_transactions_only_in_template(transactions_only_in_template, &connection)
    {
        processing::log_processing_error(&format!("Could not insert the transactions_only_in_template into the database. Skipping the remaining processing. Unclean database state! Error: {}", e));
        return;
    }
    if let Err(e) =
        db::insert_sanctioned_transaction_infos(sanctioned_transaction_infos, &connection)
    {
        processing::log_processing_error(&format!("Could not insert the sanctioned_transaction_infos into the database. Skipping the remaining processing. Unclean database state! Error: {}", e));
        return;
    }
    if let Err(e) = db::insert_conflicting_transactions(conflicting_transactions, &connection) {
        processing::log_processing_error(&format!("Could not insert the conflicting_transactions into the database. Skipping the remaining processing. Unclean database state! Error: {}", e));
        return;
    }

    let newly_sactioned_utxos = processing::build_newly_created_sanctioned_utxos(&bitcoin_block);
    if !newly_sactioned_utxos.is_empty() {
        log::info!(target: "sanctioned_utxos", "Inserting {} new sanctioned UTXOs into the database.", newly_sactioned_utxos.len());
        if let Err(e) = db::insert_sanctioned_utxos(&newly_sactioned_utxos, &connection) {
            processing::log_processing_error(&format!("Could not insert the newly_sactioned_utxos into the database. Skipping the remaining processing. Unclean database state! Error: {}", e));
            return;
        }
    }

    if let Err(e) =
        db::insert_debug_template_selection_infos(debug_template_selection_infos, &connection)
    {
        log::warn!(target: processing::LOG_TARGET_PROCESSING, "Could not insert the debug_template_selection_infos into the database. Non-critical. Error: {}", e);
        return;
    }

    last_templates.clear();
}

fn select_best_template_for_block(
    last_templates: &VecDeque<GetBlockTemplateResult>,
    block_txids: HashSet<Txid>,
) -> &GetBlockTemplateResult {
    assert!(!last_templates.is_empty());

    last_templates
        .iter()
        .min_by_key(|t| {
            let template_txids: HashSet<Txid> = t.transactions.iter().map(|t| t.txid).collect();
            let extra = block_txids.difference(&template_txids).count();
            let missing = template_txids.difference(&block_txids).count();
            missing + extra
        })
        .unwrap() // we can unwrap the Option here as we WILL find a minimum if the last_templates is not empty (which is asserted).
}

fn log_template_infos(t: &GetBlockTemplateResult) {
    log::info!(
        target: LOG_TARGET_STATS,
        "New Template based on {} | height {} | {} tx | coinbase {}",
        t.previous_block_hash,
        t.height,
        t.transactions.len(),
        t.coinbase_value
    );

    metrics::STAT_CURRENT_TEMPLATE_TRANSACTIONS_GAUGE.set(t.transactions.len() as i64);
    metrics::STAT_CURRENT_TEMPLATE_COINBASE_VALUE_GAUGE.set(t.coinbase_value.as_sat() as i64);
}

// includes an auto-generated function to identify OFAC sanctioned addresses
// the generation code can be found in build.rs
include!(concat!(env!("OUT_DIR"), "/list_sanctioned_addr.rs"));

fn scantxoutset_sanctioned_tx(
    rpc: &Client,
) -> Result<
    (
        Vec<shared_model::SanctionedUtxo>,
        shared_model::SanctionedUtxoScanInfo,
    ),
    Error,
> {
    let addrs = get_sanctioned_addresses();
    log::info!(
        target: LOG_TARGET_UTXOSETSCAN,
        "Starting UTXO set scan for Sanctioned UTXOs with {} addresses.",
        addrs.len()
    );
    let descriptors: Vec<ScanTxOutRequest> = addrs
        .iter()
        .map(|addr| ScanTxOutRequest::Single(format!("addr({})", addr)))
        .collect();
    let start = Instant::now();
    let scan_result = rpc.scan_tx_out_set_blocking(&descriptors)?;

    let duration = start.elapsed();

    log::info!(
        target: LOG_TARGET_UTXOSETSCAN,
        "Completed UTXO set scan for Sanctioned UTXOs in {} seconds: {} Sanctioned UTXOs found",
        duration.as_secs(),
        scan_result.unspents.len()
    );

    let scan_info = shared_model::SanctionedUtxoScanInfo {
        end_time: chrono::Utc::now().naive_utc(),
        end_height: scan_result.height.unwrap_or_default() as i32,
        duration_seconds: duration.as_secs() as i32,
        utxo_count: scan_result.unspents.len() as i32,
        utxo_amount: scan_result.total_amount.as_sat() as i64,
    };

    let utxos = scan_result
        .unspents
        .iter()
        .map(|utxo| {
            let mut txid_reversed = utxo.txid.to_vec();
            txid_reversed.reverse();

            shared_model::SanctionedUtxo {
                txid: txid_reversed,
                vout: utxo.vout as i32,
                script_pubkey: utxo.script_pub_key.to_bytes(),
                amount: utxo.amount.as_sat() as i64,
                height: utxo.height as i32,
            }
        })
        .collect();

    Ok((utxos, scan_info))
}

fn start_sanctioned_utxos_scan_thread(rpc_client: Client, db_pool: db_pool::PgPool) {
    thread::spawn(move || loop {
        let (sanctioned_utxos, scan_info): (
            Vec<shared_model::SanctionedUtxo>,
            shared_model::SanctionedUtxoScanInfo,
        ) = match scantxoutset_sanctioned_tx(&rpc_client) {
            Ok(scan_results) => scan_results,
            Err(e) => {
                log::error!(
                    target: LOG_TARGET_UTXOSETSCAN,
                    "Could not scan the UTXO set for sanctioned UTXOs. Retrying in {:?}. Error: {}",
                    WAIT_TIME_BETWEEN_FAILED_UTXO_SET_SCANS,
                    e
                );
                thread::sleep(WAIT_TIME_BETWEEN_FAILED_UTXO_SET_SCANS);
                continue;
            }
        };

        let conn = match db_pool.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!(
                    target: LOG_TARGET_UTXOSETSCAN,
                    "Could not get a connection from the db_pool. Retrying in {:?}. Error: {}",
                    WAIT_TIME_BETWEEN_FAILED_UTXO_SET_SCANS,
                    e
                );
                thread::sleep(WAIT_TIME_BETWEEN_FAILED_UTXO_SET_SCANS);
                continue;
            }
        };

        if let Err(err) = db::insert_sanctioned_utxo_scan_info(&scan_info, &conn) {
            log::error!(
                target: LOG_TARGET_UTXOSETSCAN,
                "Could not insert UTXO Set scan information into the database: {}",
                err
            );
        } else if let Err(err) = db::clean_and_insert_sanctioned_utxos(&sanctioned_utxos, &conn) {
            log::error!(
                target: LOG_TARGET_UTXOSETSCAN,
                "Could not insert UTXO Set scan information into the database: {}",
                err
            );
        };
        log::info!(
            target: LOG_TARGET_UTXOSETSCAN,
            "Waiting {} seconds before the next UTXO set rescan",
            WAIT_TIME_BETWEEN_UTXO_SET_SCANS.as_secs()
        );
        thread::sleep(WAIT_TIME_BETWEEN_UTXO_SET_SCANS);
    });
}

fn start_retry_unknown_pool_identification_thread(rpc_client: Client, db_pool: db_pool::PgPool) {
    thread::spawn(move || {
        let conn = match db_pool.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!(
                    target: LOG_TARGET_REIDUNKNOWNPOOLS,
                    "Could not get a connection from the db_pool. Retrying in {:?}. Error: {}",
                    WAIT_TIME_BETWEEN_FAILED_UTXO_SET_SCANS,
                    e
                );
                return;
            }
        };

        let blocks_with_unknown_pools = match db::unknown_pool_blocks(&conn) {
            Ok(blocks) => blocks,
            Err(e) => {
                log::error!(
                    target: LOG_TARGET_REIDUNKNOWNPOOLS,
                    "Could not load blocks with unknown pools: {}",
                    e
                );
                return;
            }
        };
        log::info!(
            target: LOG_TARGET_REIDUNKNOWNPOOLS,
            "Loaded {} blocks with 'Unknown' pools. Retrying pool identification.",
            blocks_with_unknown_pools.len()
        );

        for block in blocks_with_unknown_pools.iter() {
            let hash_str = hex::encode(&block.hash);

            let mut reversed_hash = block.hash.clone();
            reversed_hash.reverse();

            let hash = match bitcoin::hashes::sha256d::Hash::from_slice(&reversed_hash) {
                Ok(hash) => hash,
                Err(e) => {
                    log::error!(
                        target: LOG_TARGET_REIDUNKNOWNPOOLS,
                        "Could not convert block hash {} (height {}) to bitcoin::BlockHash: {}",
                        hash_str,
                        block.height,
                        e
                    );
                    continue;
                }
            };
            let bitcoin_block = match rpc_client.get_block(&bitcoin::BlockHash::from(hash)) {
                Ok(block) => block,
                Err(e) => {
                    log::warn!(
                        target: LOG_TARGET_REIDUNKNOWNPOOLS,
                        "Could not get block {} (height {}) from the Bitcoin Core RPC server: {}",
                        hash_str,
                        block.height,
                        e
                    );
                    break;
                }
            };

            if let Some(pool) = bitcoin_block.identify_pool() {
                match db::update_pool_name_with_block_id(&conn, block.id, &pool.name) {
                    Ok(_) => {
                        log::info!(
                            target: LOG_TARGET_REIDUNKNOWNPOOLS,
                            "Updated pool of {} to {}.",
                            bitcoin_block.block_hash(),
                            pool.name
                        );
                    }
                    Err(e) => {
                        log::error!(
                            target: LOG_TARGET_REIDUNKNOWNPOOLS,
                            "Could not update pool_name of block {:?}: {}",
                            bitcoin_block.block_hash(),
                            e
                        );
                        continue;
                    }
                };
            }
        }
        log::info!(
            target: LOG_TARGET_REIDUNKNOWNPOOLS,
            "Finished trying to re-indentify Unknown pools"
        );
    });
}

fn mempool_age_seconds(
    rpc: &Client,
    txids_only_in_template: &HashSet<&Txid>,
) -> HashMap<Txid, i32> {
    let mut txid_to_seconds_in_mempool: HashMap<Txid, i32> = HashMap::new();
    log::info!(target: processing::LOG_TARGET_PROCESSING, "Getting the mempool entry times for {} only-in-template-transactions", txids_only_in_template.len());

    // used the same time for all transactions
    let now = chrono::Local::now().timestamp();

    let mut failed_requests: usize = 0;
    for txid in txids_only_in_template.iter() {
        metrics::RUNTIME_REQUESTED_MEMPOOL_TRANSACTIONS.inc();
        let seconds: i32 = match rpc.get_mempool_entry(txid) {
            Ok(entry) => (now - entry.time as i64) as i32,
            Err(e) => {
                log::warn!(
                    target: LOG_TARGET_RPC,
                    "Getting the mempool entry time for transaction {} failed: {}",
                    txid.to_string(),
                    e
                );
                failed_requests += 1;
                -1
            }
        };
        txid_to_seconds_in_mempool.insert(**txid, seconds);
    }
    log::info!(target: processing::LOG_TARGET_PROCESSING, "Completed requesting mempool entry times for {} transactions. {} requests failed.", txids_only_in_template.len(), failed_requests);
    txid_to_seconds_in_mempool
}
