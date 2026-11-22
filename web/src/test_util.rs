//! Fixtures and helpers shared between the tests of this crate.

use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use miningpool_observer_shared::chrono::NaiveDateTime;
use miningpool_observer_shared::config::WebSiteConfig;
use miningpool_observer_shared::diesel::pg::PgConnection;
use miningpool_observer_shared::diesel::prelude::*;
use miningpool_observer_shared::model::{
    Block, DebugTemplateSelectionInfo, NewBlock, SanctionedUtxoScanInfo, Transaction,
    TransactionOnlyInBlock, TransactionOnlyInTemplate,
};
use miningpool_observer_shared::tags;
use resvg::usvg;

use crate::model::{
    ConflictingOutpoint, ConflictingTransactionSet, ConflictingTranscationInfo,
    MissingSanctionedTransaction, MissingTransactionBlockInfo,
};
use crate::util;

/// The templates the web server renders.
const TEMPLATE_GLOB: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../www/templates/**/*");
/// The fonts the OpenGraph images are rendered with.
const FONT_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../www/static/fonts");

/// A Tera instance set up like the one the web server uses.
pub fn tera() -> tera::Tera {
    let mut tera = tera::Tera::new(TEMPLATE_GLOB).expect("the templates should parse");
    tera.register_function("block_tag_id_to_tag", util::block_tag_id_to_tag());
    tera.register_function("tx_tag_id_to_tag", util::tx_tag_id_to_tag());
    tera.register_function("seconds_to_duration", util::seconds_to_duration());
    tera
}

/// usvg options set up like the ones the web server uses.
pub fn usvg_options() -> usvg::Options<'static> {
    let mut opt = usvg::Options::default();
    opt.fontdb_mut().load_fonts_dir(FONT_DIR);
    opt.fontdb_mut().set_sans_serif_family("Roboto");
    opt.fontdb_mut().set_monospace_family("Roboto Mono");
    opt
}

pub fn site_config() -> WebSiteConfig {
    WebSiteConfig {
        title: "Test Observer".to_string(),
        footer: "<p>A test footer</p>".to_string(),
        base_url: "https://observer.test".to_string(),
    }
}

/// The Bitcoin Core version string the footer shows.
pub fn node_version() -> String {
    "/Satoshi:99.0.0/".to_string()
}

/// A fixed point in time used by the fixtures: 2021-11-14 09:12:11 UTC.
pub fn time() -> NaiveDateTime {
    NaiveDateTime::parse_from_str("2021-11-14 09:12:11", "%Y-%m-%d %H:%M:%S").unwrap()
}

/// A 32 byte hash where every byte is `byte`.
pub fn hash(byte: u8) -> Vec<u8> {
    vec![byte; 32]
}

/// A block with all fields filled in.
pub fn block_fixture() -> Block {
    Block {
        id: 1,
        hash: hash(0x11),
        prev_hash: hash(0x22),
        height: 700_000,
        tags: vec![tags::BlockTag::TaprootSignaling as i32],
        missing_tx: 3,
        extra_tx: 2,
        shared_tx: 1997,
        sanctioned_missing_tx: 1,
        equality: 0.98,
        block_time: time(),
        block_seen_time: time(),
        block_tx: 1999,
        block_sanctioned: 1,
        block_cb_value: 625_000_000,
        block_cb_fees: 25_000_000,
        block_weight: 3_993_000,
        block_pkg_weights: vec![1_000, 250_000, 3_742_000],
        block_pkg_feerates: vec![120.5, 15.25, 1.0],
        pool_name: "Test Pool".to_string(),
        pool_link: "https://pool.test".to_string(),
        pool_id_method: "coinbase tag".to_string(),
        template_tx: 2000,
        template_time: time(),
        template_sanctioned: 2,
        template_cb_value: 624_000_000,
        template_cb_fees: 24_000_000,
        template_weight: 3_990_000,
        template_pkg_weights: vec![1_000, 251_000, 3_738_000],
        template_pkg_feerates: vec![121.5, 16.25, 2.0],
        template_sigops: 15_000,
        block_sigops: 14_500,
    }
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

pub fn transaction_only_in_template_fixture(
    position: i32,
    mempool_age_seconds: i32,
) -> TransactionOnlyInTemplate {
    TransactionOnlyInTemplate {
        block_id: 1,
        position,
        mempool_age_seconds,
        transaction_txid: hash(0xaa),
    }
}

pub fn transaction_only_in_block_fixture(position: i32) -> TransactionOnlyInBlock {
    TransactionOnlyInBlock {
        block_id: 1,
        position,
        transaction_txid: hash(0xbb),
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

pub fn missing_sanctioned_transaction_fixture() -> MissingSanctionedTransaction {
    MissingSanctionedTransaction {
        transaction: transaction_fixture(vec![tags::TxTag::ToSanctioned as i32]),
        missing_info: transaction_only_in_template_fixture(42, 300),
        addresses: vec!["1SanctionedAddressxxxxxxxxxxxxxxxx".to_string()],
    }
}

pub fn conflicting_transaction_info_fixture() -> ConflictingTranscationInfo {
    ConflictingTranscationInfo {
        block: block_fixture(),
        conflicting_transaction_sets: vec![ConflictingTransactionSet {
            template_transactions: vec![transaction_fixture(vec![
                tags::TxTag::Conflicting as i32,
                tags::TxTag::RbfSignaling as i32,
            ])],
            block_transactions: vec![transaction_fixture(vec![tags::TxTag::Conflicting as i32])],
            conflicting_outpoints: vec![ConflictingOutpoint {
                txid: hash(0xcc),
                vout: 1,
            }],
        }],
    }
}

pub fn sanctioned_utxo_scan_info_fixture() -> SanctionedUtxoScanInfo {
    SanctionedUtxoScanInfo {
        end_time: time(),
        end_height: 700_000,
        duration_seconds: 42,
        utxo_amount: 123_456_789,
        utxo_count: 7,
    }
}

pub fn debug_template_selection_info_fixture(selected: bool) -> DebugTemplateSelectionInfo {
    DebugTemplateSelectionInfo {
        block_id: 1,
        template_time: time(),
        count_missing: 3,
        count_shared: 1997,
        count_extra: 2,
        selected,
    }
}

//##### DATABASE

/// The migrations that set up the schema of the test database.
pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("../migrations/");

/// A connection to the database given in the TEST_DATABASE_URL environment
/// variable, with the migrations applied. Returns None and prints a note when
/// the variable isn't set, so the tests using it can skip themselves.
///
/// The connection is in a test transaction: nothing a test writes is committed
/// and tests don't see each others writes.
pub fn test_conn() -> Option<PgConnection> {
    let url = match std::env::var("TEST_DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            println!("skipped: TEST_DATABASE_URL not set");
            return None;
        }
    };

    // The migrations only need to run once per test binary. Running them from
    // multiple threads at the same time can fail.
    MIGRATIONS_RUN.call_once(|| {
        let mut conn =
            PgConnection::establish(&url).expect("should be able to connect to TEST_DATABASE_URL");
        conn.run_pending_migrations(MIGRATIONS)
            .expect("should be able to run the migrations");
    });

    let mut conn =
        PgConnection::establish(&url).expect("should be able to connect to TEST_DATABASE_URL");
    conn.begin_test_transaction()
        .expect("should be able to start a test transaction");
    Some(conn)
}

static MIGRATIONS_RUN: std::sync::Once = std::sync::Once::new();

/// A block to insert into the database. Has the same values as
/// [`block_fixture`], but no id: the database assigns that.
pub fn new_block_fixture() -> NewBlock {
    let b = block_fixture();
    NewBlock {
        hash: b.hash,
        prev_hash: b.prev_hash,
        height: b.height,
        tags: b.tags,
        missing_tx: b.missing_tx,
        extra_tx: b.extra_tx,
        shared_tx: b.shared_tx,
        sanctioned_missing_tx: b.sanctioned_missing_tx,
        equality: b.equality,
        block_time: b.block_time,
        block_seen_time: b.block_seen_time,
        block_tx: b.block_tx,
        block_sanctioned: b.block_sanctioned,
        block_cb_value: b.block_cb_value,
        block_cb_fees: b.block_cb_fees,
        block_weight: b.block_weight,
        block_pkg_weights: b.block_pkg_weights,
        block_pkg_feerates: b.block_pkg_feerates,
        pool_name: b.pool_name,
        pool_link: b.pool_link,
        pool_id_method: b.pool_id_method,
        template_tx: b.template_tx,
        template_time: b.template_time,
        template_sanctioned: b.template_sanctioned,
        template_cb_value: b.template_cb_value,
        template_cb_fees: b.template_cb_fees,
        template_weight: b.template_weight,
        template_pkg_weights: b.template_pkg_weights,
        template_pkg_feerates: b.template_pkg_feerates,
        template_sigops: b.template_sigops,
        block_sigops: b.block_sigops,
    }
}
