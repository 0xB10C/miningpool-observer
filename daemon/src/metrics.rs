use lazy_static::lazy_static;
use prometheus::{self, IntCounter, IntGauge};
use prometheus::{register_int_counter, register_int_gauge};

// Prometheus Metrics

const PREFIX: &str = "miningpoolobserver_daemon";

lazy_static! {

    // -------------------- Runtime

    /// Start time as UNIX timestamp. Changes can be used to detect and alert unplanned restarts.
    pub static ref RUNTIME_START_TIME: IntGauge =
        register_int_gauge!(format!("{}_runtime_start_timestamp", PREFIX), "UNIX timestamp at the time when the daemon started.").unwrap();

    /// Number of templates requested from Bitcoin Core.
    pub static ref RUNTIME_REQUESTED_TEMPLATES: IntCounter =
        register_int_counter!(format!("{}_runtime_requested_templates", PREFIX), "Number of new templates requested from Bitcoin Core.").unwrap();

    /// Number of blocks requested from Bitcoin Core.
    pub static ref RUNTIME_REQUESTED_BLOCKS: IntCounter =
        register_int_counter!(format!("{}_runtime_requested_blocks", PREFIX), "Number of blocks requested from Bitcoin Core.").unwrap();

    /// Number of transactions requested from the mempool.
    pub static ref RUNTIME_REQUESTED_MEMPOOL_TRANSACTIONS: IntCounter =
        register_int_counter!(format!("{}_runtime_requested_mempool_transactions", PREFIX), "Number transactions looked up in the mempool.").unwrap();

    /// Number of transactions requested from the mempool.
    pub static ref RUNTIME_SKIPPED_BLOCK_EVENTS: IntCounter =
        register_int_counter!(format!("{}_runtime_skipped_block_events", PREFIX), "Number of block-skipped-events. Can happen if there are multiple rapid blocks.").unwrap();

    /// Number of templates currently in memory.
    pub static ref RUNTIME_TEMPLATES_IN_MEMORY: IntGauge =
        register_int_gauge!(format!("{}_runtime_templates_in_memory", PREFIX), "Number of templates kept in memory.").unwrap();

    // -------------------- Template Statistics

    /// Number of transactions in the most recently queried block template.
    pub static ref STAT_CURRENT_TEMPLATE_TRANSACTIONS_GAUGE: IntGauge =
        register_int_gauge!(format!("{}_stats_current_template_transactions", PREFIX), "Number of transactions in the current block template.").unwrap();

    /// Coinbase value of the most recently queried block template.
    pub static ref STAT_CURRENT_TEMPLATE_COINBASE_VALUE_GAUGE: IntGauge =
        register_int_gauge!(format!("{}_stats_current_template_coinbase_value_sat", PREFIX), "Output value of the coinbase transaction in the block template.").unwrap();

    /// Sigops in the most recently queried block template.
    pub static ref STAT_CURRENT_TEMPLATE_SIGOPS_GAUGE: IntGauge =
        register_int_gauge!(format!("{}_stats_current_template_sigops", PREFIX), "Sigops of the transactions in the block template.").unwrap();

    /// Number of conflicting transaction sets between template and block.
    pub static ref STAT_CONFLICTING_TRANSACTION_SETS: IntCounter =
        register_int_counter!(format!("{}_stats_conflicting_transaction_sets", PREFIX), "Total number of processed conflicting transaction sets.").unwrap();

    /// Number of transactions requested from the mempool.
    pub static ref STAT_SANCTIONED_TRANSACTIONS: IntCounter =
        register_int_counter!(format!("{}_stats_sanctioned_transactions", PREFIX), "Total number sanctioned transactions processed.").unwrap();


    // -------------------- Errors

    /// Number of Bitcoin Core RPC errors. Can be used for alerting.
    pub static ref ERROR_RPC: IntCounter =
        register_int_counter!(format!("{}_error_rpc_failed", PREFIX), "Number of failed RPC calls.").unwrap();

    /// Number of processing errors. Can be used for alerting.
    pub static ref ERROR_PROCESSING: IntCounter =
        register_int_counter!(format!("{}_error_processing", PREFIX), "Number of processing errors.").unwrap();

    /// Number of database connection pool errors. Can be used for alerting.
    pub static ref ERROR_DBPOOL: IntCounter =
        register_int_counter!(format!("{}_error_db_pool", PREFIX), "Number of database connection pool errors.").unwrap();
}
