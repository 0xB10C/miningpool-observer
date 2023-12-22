#![cfg_attr(feature = "strict", deny(warnings))]

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Display;
use std::fs::File;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time;
use std::time::Instant;

use miningpool_observer_shared::bitcoincore_rpc::bitcoin;
use miningpool_observer_shared::bitcoincore_rpc::bitcoin::{
    address::NetworkUnchecked, address::ParseError, hash_types::Txid, hashes::Hash, Address,
    Amount, Block, Network,
};

use bitcoin_pool_identification::{parse_json, PoolIdentification, DEFAULT_MAINNET_POOL_LIST};
use simple_logger::SimpleLogger;

use miningpool_observer_shared::bitcoincore_rpc::json::{
    GetBlockTemplateModes, GetBlockTemplateResult, GetBlockTemplateRules, GetBlockTxFeesResult,
    ScanTxOutRequest,
};
use miningpool_observer_shared::bitcoincore_rpc::jsonrpc;
use miningpool_observer_shared::bitcoincore_rpc::{Auth, Client, Error, RpcApi};
use miningpool_observer_shared::chrono;
use miningpool_observer_shared::diesel;
use miningpool_observer_shared::{
    config, db_pool, model as shared_model, prometheus_metric_server,
};

use crate::model::TxInfo;

mod db;
mod metrics;
mod model;
mod processing;

const WAIT_TIME_BETWEEN_TEMPLATE_QUERIES: time::Duration = time::Duration::from_secs(10);
const WAIT_TIME_BETWEEN_CONNPOOL_GETCONNECTION: time::Duration = time::Duration::from_secs(1);
const WAIT_TIME_BETWEEN_UTXO_SET_SCANS: time::Duration = time::Duration::from_secs(60 * 60 * 3); // 3 hours
const WAIT_TIME_BETWEEN_FAILED_UTXO_SET_SCANS: time::Duration = time::Duration::from_secs(60 * 5); // 5 minutes
const WAIT_TIME_BETWEEN_SANCTIONED_ADDRESS_UPDATES: time::Duration =
    time::Duration::from_secs(60 * 60 * 24); // 1 day
const TIMEOUT_UTXO_SET_SCANS: time::Duration = time::Duration::from_secs(60 * 8); // 8 minutes
const MAX_OLD_TEMPLATES: usize = 15;
const TIMEOUT_HTTP_GET_REQUEST: u64 = 10; // seconds

const LOG_TARGET_RPC: &str = "rpc";
const LOG_TARGET_REIDUNKNOWNPOOLS: &str = "re-id_unknown_pools";
const LOG_TARGET_UTXOSETSCAN: &str = "utxo_set_scan";
const LOG_TARGET_STATS: &str = "stats";
const LOG_TARGET_DBPOOL: &str = "dbpool";
const LOG_TARGET_STARTUP: &str = "startup";
const LOG_TARGET_RETAG_TX: &str = "retagtx";
const LOG_TARGET_UPDATE_SANCTIONED_ADDRESSES: &str = "sanctionupdate";

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

    match SimpleLogger::new()
        .with_utc_timestamps()
        .with_level(config.log_level)
        .init()
    {
        Ok(_) => (),
        Err(e) => panic!("Could not setup logger: {}", e),
    }

    let rpc_client = match Client::new(&config.rpc_url.clone(), config.rpc_auth.clone()) {
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

    // Sanctioned addresses updater
    // Download the list before we start and then periodically update it.
    if let Err(e) = start_sanctioned_addresses_updater_thread(
        config.sanctioned_addresses_url,
        conn_pool.clone(),
    ) {
        panic!(
            "During startup: Could not update the sanctioned address list: {}",
            e
        );
    }

    // Sanctioned UTXO (re)scan
    let utxo_set_scan_conn_pool = match db_pool::new(&config.database_url) {
        Ok(pool) => pool,
        Err(e) => panic!(
            "During startup: Could not create a Postgres connection pool: {}",
            e
        ),
    };

    let user_pass = match get_user_pass(&config.rpc_auth) {
        Ok(x) => x,
        Err(e) => panic!(
            "During startup: Could not extract the Bitcoin Core RPC credentials: {}",
            e
        ),
    };

    // Mining pool identification
    let miningpool_identification_data: model::SharedPoolIDData =
        Arc::new(Mutex::new(parse_json(DEFAULT_MAINNET_POOL_LIST)));

    // Build a custom transport here to be able to configure the timeout.
    let custom_timeout_transport = jsonrpc::simple_http::Builder::new()
        .url(&config.rpc_url.clone())
        .expect("invalid rpc url")
        .auth(user_pass.0.expect("rpc user is empty"), user_pass.1)
        .timeout(TIMEOUT_UTXO_SET_SCANS)
        .build();
    let custom_timeout_rpc_client =
        jsonrpc::client::Client::with_transport(custom_timeout_transport);

    let rpc_client_utxo_set_scan = Client::from_jsonrpc(custom_timeout_rpc_client);
    start_sanctioned_utxos_scan_thread(rpc_client_utxo_set_scan, utxo_set_scan_conn_pool);

    // Retry pool identification for "Unkown" pools
    let reid_conn_pool = match db_pool::new(&config.database_url) {
        Ok(pool) => pool,
        Err(e) => panic!(
            "During startup: Could not create a Postgres connection pool: {}",
            e
        ),
    };
    let reid_rpc_client = match Client::new(&config.rpc_url.clone(), config.rpc_auth.clone()) {
        Ok(config) => config,
        Err(e) => panic!(
            "During startup: Could not setup the Bitcoin Core RPC client: {}",
            e
        ),
    };
    start_retry_unknown_pool_identification_thread(
        reid_rpc_client,
        reid_conn_pool,
        miningpool_identification_data.clone(),
    );

    match rpc_client.get_network_info() {
        Ok(network_info) => {
            let version = network_info.subversion.replace('/', "").replace(':', " ");
            log::info!(
                target: LOG_TARGET_STARTUP,
                "Block templates are generated by Bitcoin Core with version {}.",
                version
            );
            match conn_pool.get() {
                Ok(mut conn) => {
                    if let Err(e) = db::update_node_info(&version, &mut conn) {
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

    if config.retag_transactions {
        let retag_conn_pool = match db_pool::new(&config.database_url) {
            Ok(pool) => pool,
            Err(e) => panic!(
                "During startup: Could not create a Postgres connection pool: {}",
                e
            ),
        };
        let retag_rpc_client = match Client::new(&config.rpc_url.clone(), config.rpc_auth.clone()) {
            Ok(config) => config,
            Err(e) => panic!(
                "During startup: Could not setup the Bitcoin Core RPC client: {}",
                e
            ),
        };
        retag_transactions(retag_rpc_client, retag_conn_pool);
    }

    main_loop(&rpc_client, &conn_pool, miningpool_identification_data);
}

fn startup_db_mirgation(conn_pool: &db_pool::PgPool) {
    match conn_pool.get() {
        Ok(mut conn) => {
            match db::run_migrations(&mut conn) {
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

fn main_loop(rpc: &Client, db_pool: &db_pool::PgPool, pools: model::SharedPoolIDData) {
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
                    log::error!(
                        target: processing::LOG_TARGET_PROCESSING,
                        "Skipping the processing of block {}.",
                        current_template.previous_block_hash
                    );
                    metrics::ERROR_RPC.inc();
                    thread::sleep(WAIT_TIME_BETWEEN_TEMPLATE_QUERIES);
                    continue;
                }
            };

            log::info!(
                target: LOG_TARGET_STATS,
                "Processing missed block {} mined by {}",
                bitcoin_block.block_hash(),
                match bitcoin_block.identify_pool(Network::Bitcoin, &pools.lock().unwrap()) {
                    Some(result) => result.pool.name,
                    None => "UNKNOWN".to_string(),
                }
            );

            process(
                rpc,
                db_pool,
                &bitcoin_block,
                &block_tx_fees,
                &mut last_templates,
                pools.clone(),
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
                log::error!(
                    target: processing::LOG_TARGET_PROCESSING,
                    "Skipping the processing of block {}.",
                    current_template.previous_block_hash
                );
                metrics::ERROR_RPC.inc();
                thread::sleep(WAIT_TIME_BETWEEN_TEMPLATE_QUERIES);
                continue;
            }
        };

        log::info!(
            target: LOG_TARGET_STATS,
            "New block detected {} mined by {}",
            bitcoin_block.block_hash(),
            match bitcoin_block.identify_pool(Network::Bitcoin, &pools.lock().unwrap()) {
                Some(result) => result.pool.name,
                None => "UNKNOWN".to_string(),
            }
        );

        process(
            rpc,
            db_pool,
            &bitcoin_block,
            &block_tx_fees,
            &mut last_templates,
            pools.clone(),
        );
    }
}

fn process(
    rpc: &Client,
    db_pool: &db_pool::PgPool,
    bitcoin_block: &Block,
    block_tx_fees: &GetBlockTxFeesResult,
    last_templates: &mut VecDeque<GetBlockTemplateResult>,
    pools: model::SharedPoolIDData,
) {
    let block_tx_data = processing::build_block_tx_data(bitcoin_block, block_tx_fees);

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
    let template = select_best_template_for_block(last_templates, block_tx_data.txids.clone());

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

    let mut connection = &mut match db_pool.get() {
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
                    log::error!(target: LOG_TARGET_DBPOOL, "Could not get a connection from the connection pool. Skipping processing of block {}: {}", bitcoin_block.block_hash(), e);
                    metrics::ERROR_DBPOOL.inc();
                    return;
                }
            }
        }
    };

    let sanctioned_addresses: HashSet<String> = match db::sanctioned_addresses(&mut connection) {
        Ok(addresses) => addresses.iter().map(|a| a.address.clone()).collect(),
        Err(e) => {
            processing::log_processing_error(&format!("Could not load the sanctioned addresses from the database. Using empty list of addresses: {}", e));
            HashSet::new()
        }
    };

    let sanctioned_utxos = match db::get_sanctioned_utxos(&mut connection) {
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
        &sanctioned_addresses,
    );

    let block_fees: Amount =
        Amount::from_sat(block_tx_fees.tx.iter().map(|tx| tx.fee.to_sat()).sum());
    let template_fees: Amount =
        Amount::from_sat(template.transactions.iter().map(|tx| tx.fee.to_sat()).sum());
    let missing_tx: i32 = txids_only_in_template.len() as i32;
    let extra_tx: i32 = txids_only_in_block.len() as i32;
    let block = processing::build_block(
        bitcoin_block,
        template,
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
        &sanctioned_addresses,
        pools,
    );

    let block_id = match db::insert_block(&block, &mut connection) {
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
        &sanctioned_addresses,
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
        &sanctioned_addresses,
    );
    let transactions_only_in_block = processing::build_transactions_only_in_block(
        block_id,
        &txids_only_in_block,
        &block_tx_data.txid_to_txinfo_map,
        &mut transactions,
        &outpoint_to_sanctioned_utxo_map,
        &sanctioned_addresses,
    );
    let sanctioned_transaction_infos = processing::build_sanctioned_transaction_infos(
        block_id,
        &block_tx_data,
        &template_tx_data.txids,
        &template_tx_data.txid_to_txinfo_map,
        &txids_only_in_block,
        &outpoint_to_sanctioned_utxo_map,
        &sanctioned_addresses,
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
        last_templates,
        block_tx_data.txids,
        template.current_time,
    );

    if let Err(e) =
        db::insert_transactions(transactions.values().cloned().collect(), &mut connection)
    {
        processing::log_processing_error(&format!("Could not insert the transactions into the database. Skipping the remaining processing. Unclean database state! Error: {}", e));
        return;
    }
    if let Err(e) =
        db::insert_transactions_only_in_block(transactions_only_in_block, &mut connection)
    {
        processing::log_processing_error(&format!("Could not insert the transactions_only_in_block into the database. Skipping the remaining processing. Unclean database state! Error: {}", e));
        return;
    }
    if let Err(e) =
        db::insert_transactions_only_in_template(transactions_only_in_template, &mut connection)
    {
        processing::log_processing_error(&format!("Could not insert the transactions_only_in_template into the database. Skipping the remaining processing. Unclean database state! Error: {}", e));
        return;
    }
    if let Err(e) =
        db::insert_sanctioned_transaction_infos(sanctioned_transaction_infos, &mut connection)
    {
        processing::log_processing_error(&format!("Could not insert the sanctioned_transaction_infos into the database. Skipping the remaining processing. Unclean database state! Error: {}", e));
        return;
    }
    if let Err(e) = db::insert_conflicting_transactions(conflicting_transactions, &mut connection) {
        processing::log_processing_error(&format!("Could not insert the conflicting_transactions into the database. Skipping the remaining processing. Unclean database state! Error: {}", e));
        return;
    }

    let newly_sactioned_utxos =
        processing::build_newly_created_sanctioned_utxos(bitcoin_block, &sanctioned_addresses);
    if !newly_sactioned_utxos.is_empty() {
        log::info!(target: "sanctioned_utxos", "Inserting {} new sanctioned UTXOs into the database.", newly_sactioned_utxos.len());
        if let Err(e) = db::insert_sanctioned_utxos(&newly_sactioned_utxos, &mut connection) {
            processing::log_processing_error(&format!("Could not insert the newly_sactioned_utxos into the database. Skipping the remaining processing. Unclean database state! Error: {}", e));
            return;
        }
    }

    if let Err(e) =
        db::insert_debug_template_selection_infos(debug_template_selection_infos, &mut connection)
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
    let template_sigops = t
        .transactions
        .iter()
        .map(|tx| tx.sigops as u64)
        .sum::<u64>();
    log::info!(
        target: LOG_TARGET_STATS,
        "New Template based on {} | height {} | {} tx | coinbase {} | sigops {}",
        t.previous_block_hash,
        t.height,
        t.transactions.len(),
        t.coinbase_value,
        template_sigops,
    );

    metrics::STAT_CURRENT_TEMPLATE_TRANSACTIONS_GAUGE.set(t.transactions.len() as i64);
    metrics::STAT_CURRENT_TEMPLATE_COINBASE_VALUE_GAUGE.set(t.coinbase_value.to_sat() as i64);
    metrics::STAT_CURRENT_TEMPLATE_SIGOPS_GAUGE.set(template_sigops as i64);
}

fn scantxoutset_sanctioned_tx(
    addrs: Vec<String>,
    rpc: &Client,
) -> Result<
    (
        Vec<shared_model::SanctionedUtxo>,
        shared_model::SanctionedUtxoScanInfo,
    ),
    Error,
> {
    log::info!(
        target: LOG_TARGET_UTXOSETSCAN,
        "Starting UTXO set scan for Sanctioned UTXOs with {} addresses.",
        addrs.len()
    );

    let descriptors: Vec<ScanTxOutRequest> = addrs
        .iter()
        .filter(|addr| {
            let address: Result<Address<NetworkUnchecked>, ParseError> = addr.parse();
            match address {
                Ok(_) => true,
                Err(e) => {
                    log::warn!("Could not parse address='{}' - skipping: {}", addr, e);
                    false
                }
            }
        })
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
        utxo_amount: scan_result.total_amount.to_sat() as i64,
    };

    let utxos = scan_result
        .unspents
        .iter()
        .map(|utxo| {
            let mut txid_reversed = utxo.txid.to_byte_array().to_vec();
            txid_reversed.reverse();

            shared_model::SanctionedUtxo {
                txid: txid_reversed,
                vout: utxo.vout as i32,
                script_pubkey: utxo.script_pub_key.to_bytes(),
                amount: utxo.amount.to_sat() as i64,
                height: utxo.height as i32,
            }
        })
        .collect();

    Ok((utxos, scan_info))
}

fn start_sanctioned_utxos_scan_thread(rpc_client: Client, db_pool: db_pool::PgPool) {
    thread::spawn(move || loop {
        let mut conn = &mut match db_pool.get() {
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

        let sanctioned_addresses: Vec<String> = match db::sanctioned_addresses(&mut conn) {
            Ok(sa) => sa.iter().map(|a| a.address.clone()).collect(),
            Err(e) => {
                log::error!(
                    target: LOG_TARGET_UTXOSETSCAN,
                    "Could not load sanctioned addressses from the database. Retrying in {:?}. Error: {}",
                    WAIT_TIME_BETWEEN_FAILED_UTXO_SET_SCANS,
                    e
                );
                thread::sleep(WAIT_TIME_BETWEEN_FAILED_UTXO_SET_SCANS);
                continue;
            }
        };

        let (sanctioned_utxos, scan_info): (
            Vec<shared_model::SanctionedUtxo>,
            shared_model::SanctionedUtxoScanInfo,
        ) = match scantxoutset_sanctioned_tx(sanctioned_addresses, &rpc_client) {
            Ok(scan_results) => scan_results,
            Err(e) => {
                log::error!(
                    target: LOG_TARGET_UTXOSETSCAN,
                    "Could not scan the UTXO set for sanctioned UTXOs. Retrying in {:?}. Error: {}",
                    WAIT_TIME_BETWEEN_FAILED_UTXO_SET_SCANS,
                    e
                );
                metrics::ERROR_RPC.inc();
                thread::sleep(WAIT_TIME_BETWEEN_FAILED_UTXO_SET_SCANS);
                continue;
            }
        };

        if let Err(err) = db::insert_sanctioned_utxo_scan_info(&scan_info, &mut conn) {
            log::error!(
                target: LOG_TARGET_UTXOSETSCAN,
                "Could not insert UTXO Set scan information into the database: {}",
                err
            );
        } else if let Err(err) = db::clean_and_insert_sanctioned_utxos(&sanctioned_utxos, &mut conn)
        {
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

fn retag_transactions(rpc_client: Client, db_pool: db_pool::PgPool) {
    thread::spawn(move || {
        log::info!(
            target: LOG_TARGET_RETAG_TX,
            "Starting to retag transactions. This might take a while.",
        );
        let mut conn = &mut match db_pool.get() {
            Ok(c) => c,
            Err(e) => {
                log::error!(
                    target: LOG_TARGET_RETAG_TX,
                    "Could not get a connection from the db_pool. Error: {}",
                    e
                );
                return;
            }
        };
        let transactions_in_db = match db::all_transactions(&mut conn) {
            Ok(blocks) => blocks,
            Err(e) => {
                log::error!(
                    target: LOG_TARGET_RETAG_TX,
                    "Could not load transactions from database: {}",
                    e
                );
                return;
            }
        };
        let sanctioned_addresses = match db::sanctioned_addresses(&mut conn) {
            Ok(addresses) => addresses.iter().map(|a| a.address.clone()).collect(),
            Err(e) => {
                log::error!(
                    target: LOG_TARGET_RETAG_TX,
                    "Could not load sanctioned addresses from database: {}",
                    e
                );
                return;
            }
        };
        let tx_in_db_count = transactions_in_db.len();
        log::info!(
            target: LOG_TARGET_RETAG_TX,
            "Retagging {} transactions in the database",
            tx_in_db_count,
        );
        let mut counter = 0;
        for tx_in_db in transactions_in_db {
            let mut reversed_txid = tx_in_db.txid.clone();
            reversed_txid.reverse();
            let hash = bitcoin::hashes::sha256d::Hash::from_slice(&reversed_txid).unwrap();
            let txid: Txid = Txid::from_raw_hash(hash);
            if let Ok(tx) = rpc_client.get_raw_transaction(&txid, None) {
                let tx_info = TxInfo {
                    txid,
                    tx: tx.clone(),
                    pos: -1,
                    fee: Amount::from_sat(tx_in_db.fee as u64),
                };
                let mut old_tags = tx_in_db.tags.clone();
                old_tags.sort();
                let mut new_tags =
                    processing::retag_transaction(&tx, &tx_info, &sanctioned_addresses);
                for tag in old_tags.iter() {
                    if !new_tags.contains(tag) {
                        new_tags.push(*tag);
                    }
                }
                new_tags.sort();

                // we can compare them here, as they are both sorted
                if new_tags != old_tags {
                    match db::update_transaction_tags(&new_tags, &tx_in_db.txid.clone(), &mut conn)
                    {
                        Ok(()) => {
                            log::info!(
                                target: LOG_TARGET_RETAG_TX,
                                "Retagged transaction {}: old={:?} new={:?}",
                                hex::encode(tx_in_db.txid.clone()),
                                old_tags,
                                new_tags,
                            );
                        }
                        Err(e) => {
                            log::info!(
                                target: LOG_TARGET_RETAG_TX,
                                "Could not retagged transaction {}: old={:?} new={:?}, error={}",
                                hex::encode(tx_in_db.txid.clone()),
                                old_tags,
                                new_tags,
                                e,
                            );
                        }
                    };
                };
                counter += 1;
                if counter % 100 == 0 {
                    log::info!(
                        target: LOG_TARGET_RETAG_TX,
                        "Retagged {} out of {} transactions",
                        counter,
                        tx_in_db_count,
                    );
                }
            }
        }
    });
}

#[derive(Debug)]
enum UpdateSanctionedAddressesError {
    Database(diesel::result::Error),
    MinReq(minreq::Error),
    DBPool(String),
    HTTP(String),
}

impl Display for UpdateSanctionedAddressesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateSanctionedAddressesError::DBPool(e) => write!(f, "{}", e),
            UpdateSanctionedAddressesError::Database(e) => write!(f, "{}", e),
            UpdateSanctionedAddressesError::MinReq(e) => write!(f, "{}", e),
            UpdateSanctionedAddressesError::HTTP(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for UpdateSanctionedAddressesError {}

impl From<minreq::Error> for UpdateSanctionedAddressesError {
    fn from(err: minreq::Error) -> Self {
        UpdateSanctionedAddressesError::MinReq(err)
    }
}

impl From<diesel::result::Error> for UpdateSanctionedAddressesError {
    fn from(err: diesel::result::Error) -> Self {
        UpdateSanctionedAddressesError::Database(err)
    }
}

fn update_sanctioned_addresses(
    sanctioned_addresses_url: String,
    db_pool: &db_pool::PgPool,
) -> Result<(), UpdateSanctionedAddressesError> {
    let response = minreq::get(sanctioned_addresses_url.clone())
        .with_timeout(TIMEOUT_HTTP_GET_REQUEST)
        .send()?;

    if response.status_code >= 400 {
        log::error!(
            target: LOG_TARGET_UPDATE_SANCTIONED_ADDRESSES,
            "Failed to load sanctioned addresses from {}: {} {}",
            sanctioned_addresses_url,
            response.status_code,
            response.reason_phrase,
        );
        return Err(UpdateSanctionedAddressesError::HTTP(format!(
            "{} returned: {} {}",
            sanctioned_addresses_url, response.status_code, response.reason_phrase
        )));
    }

    let addresses: Vec<shared_model::SanctionedAddress> = response
        .as_str()?
        .lines()
        .map(|a| shared_model::SanctionedAddress {
            address: a.to_string(),
        })
        .filter(|addr| {
            let address: Result<Address<NetworkUnchecked>, ParseError> = addr.address.parse();
            match address {
                Ok(_) => true,
                Err(e) => {
                    log::warn!("While updating sanctioned addresses: Could not parse address='{}' - skipping: {}", addr.address, e);
                    false
                }
            }
        })
        .collect();

    log::info!(
        target: LOG_TARGET_UPDATE_SANCTIONED_ADDRESSES,
        "Loaded {} sanctioned addresses from {}",
        addresses.len(),
        sanctioned_addresses_url
    );

    let mut conn = match db_pool.get() {
        Ok(c) => c,
        Err(e) => {
            log::error!(
                target: LOG_TARGET_UPDATE_SANCTIONED_ADDRESSES,
                "Could not get a connection from the connection pool: {}",
                e
            );
            return Err(UpdateSanctionedAddressesError::DBPool(e.to_string()));
        }
    };

    match db::replace_sanctioned_addresses(addresses, &mut conn) {
        Ok(()) => {
            log::info!(
                target: LOG_TARGET_UPDATE_SANCTIONED_ADDRESSES,
                "Replaced the sanctioned addresses in the database.",
            );
        }
        Err(e) => {
            log::error!(
                target: LOG_TARGET_RETAG_TX,
                "Could not replace the sanctioned addresses in the database: {}",
                e,
            );
            return Err(e.into());
        }
    };

    return Ok(());
}

fn start_sanctioned_addresses_updater_thread(
    url: String,
    db_pool: db_pool::PgPool,
) -> Result<(), UpdateSanctionedAddressesError> {
    // first do one update during start-up
    update_sanctioned_addresses(url.clone(), &db_pool.clone())?;
    // and then periodcially
    thread::spawn(move || loop {
        thread::sleep(WAIT_TIME_BETWEEN_SANCTIONED_ADDRESS_UPDATES);

        if let Err(e) = update_sanctioned_addresses(url.clone(), &db_pool) {
            log::error!(
                target: LOG_TARGET_UPDATE_SANCTIONED_ADDRESSES,
                "Could not update sanctioned address list from {}: {}",
                url,
                e
            );
        }
    });
    Ok(())
}

fn start_retry_unknown_pool_identification_thread(
    rpc_client: Client,
    db_pool: db_pool::PgPool,
    pools: model::SharedPoolIDData,
) {
    thread::spawn(move || {
        let mut conn = match db_pool.get() {
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
        let blocks_with_unknown_pools = match db::unknown_pool_blocks(&mut conn) {
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

            if let Some(result) =
                bitcoin_block.identify_pool(Network::Bitcoin, &pools.lock().unwrap())
            {
                match db::update_pool_name_with_block_id(&mut conn, block.id, &result.pool.name) {
                    Ok(_) => {
                        log::info!(
                            target: LOG_TARGET_REIDUNKNOWNPOOLS,
                            "Updated pool of {} to {}.",
                            bitcoin_block.block_hash(),
                            result.pool.name
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

/// Convert into the arguments that jsonrpc::Client needs.
/// adopted from https://docs.rs/bitcoincore-rpc/0.14.0/src/bitcoincore_rpc/client.rs.html#1122-1143
fn get_user_pass(
    auth: &Auth,
) -> miningpool_observer_shared::bitcoincore_rpc::Result<(Option<String>, Option<String>)> {
    match auth {
        Auth::None => Ok((None, None)),
        Auth::UserPass(u, p) => Ok((Some(u.to_string()), Some(p.to_string()))),
        Auth::CookieFile(path) => {
            let mut file = File::open(path)?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)?;
            let mut split = contents.splitn(2, ':');
            Ok((
                Some(
                    split
                        .next()
                        .ok_or(
                            miningpool_observer_shared::bitcoincore_rpc::Error::InvalidCookieFile,
                        )?
                        .into(),
                ),
                Some(
                    split
                        .next()
                        .ok_or(
                            miningpool_observer_shared::bitcoincore_rpc::Error::InvalidCookieFile,
                        )?
                        .into(),
                ),
            ))
        }
    }
}

fn mempool_age_seconds(
    rpc: &Client,
    txids_only_in_template: &HashSet<&Txid>,
) -> HashMap<Txid, i32> {
    let mut txid_to_seconds_in_mempool: HashMap<Txid, i32> = HashMap::new();
    log::info!(
        target: processing::LOG_TARGET_PROCESSING,
        "Getting the mempool entry times for {} only-in-template-transactions",
        txids_only_in_template.len()
    );

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
    log::info!(
        target: processing::LOG_TARGET_PROCESSING,
        "Completed requesting mempool entry times for {} transactions. {} requests failed.",
        txids_only_in_template.len(),
        failed_requests
    );
    txid_to_seconds_in_mempool
}
