//! Renders every template the request handlers render, with a context built
//! the same way the handlers build it. A template that fails to render results
//! in an error page (or a broken OpenGraph image) in production.

use miningpool_observer_shared::tags;
use miningpool_observer_shared::tags::THRESHOLD_TRANSACTION_CONSIDERED_YOUNG;
use serde::Serialize;

use crate::db::{MAX_BLOCKS_PER_PAGE, MAX_UTXOSET_SCANS_PER_PAGE};
use crate::model::{AvgPoolFees, BlockWithTx, PoolSanctionedTableEntry};
use crate::model::{DebugTemplateSelectionInfosAndBlock, MissingTransaction};
use crate::ogimage::render_and_encode;
use crate::test_util::*;

const QUERY_PAGE: &str = "page";
const QUERY_POOL: &str = "pool";

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

/// Renders a template and fails with the full Tera error chain if it doesn't.
fn render(template: &str, ctx: &tera::Context) -> String {
    match tera().render(template, ctx) {
        Ok(rendered) => rendered,
        Err(e) => {
            let mut message = format!("could not render {}: {}", template, e);
            let mut source = std::error::Error::source(&e);
            while let Some(e) = source {
                message.push_str(&format!("\n  caused by: {}", e));
                source = e.source();
            }
            panic!("{}", message);
        }
    }
}

/// The context keys every page sets.
fn base_context() -> tera::Context {
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", &site_config());
    ctx.insert("NODE_VERSION", &node_version());
    ctx
}

/// Asserts that the page has the parts every page shares.
fn assert_is_a_page(rendered: &str) {
    assert!(
        rendered.contains("Test Observer"),
        "the site title is shown"
    );
    assert!(rendered.contains("Satoshi:99.0.0"), "the node version");
    assert!(rendered.contains("A test footer"), "the configured footer");
}

fn block_hash_hex() -> String {
    hex::encode(hash(0x11))
}

fn txid_hex() -> String {
    hex::encode(hash(0xaa))
}

//##### INDEX

#[test]
fn test_render_index() {
    let rendered = render("index.html", &base_context());
    assert_is_a_page(&rendered);
}

//##### TEMPLATE & BLOCK

fn templates_and_blocks_context(
    blocks: Vec<miningpool_observer_shared::model::Block>,
    max_pages: u32,
    current_page: u32,
    current_pool: &str,
) -> tera::Context {
    let mut ctx = base_context();
    ctx.insert("MAX_BLOCKS_PER_PAGE", &MAX_BLOCKS_PER_PAGE);
    ctx.insert("NAV_PAGE_BLOCKS", &true);
    ctx.insert("QUERY_PAGE", &QUERY_PAGE);
    ctx.insert("QUERY_POOL", &QUERY_POOL);
    ctx.insert("blocks", &blocks);
    ctx.insert("MAX_PAGES", &max_pages);
    ctx.insert("CURRENT_PAGE", &current_page);
    ctx.insert("CURRENT_POOL", &current_pool);
    ctx.insert(
        "POOLS",
        &vec!["Test Pool".to_string(), "Unknown".to_string()],
    );
    ctx
}

#[test]
fn test_render_templates_and_blocks() {
    let ctx = templates_and_blocks_context(vec![block_fixture()], 2, 1, "");
    let rendered = render("templates_and_blocks.html", &ctx);

    assert_is_a_page(&rendered);
    assert!(rendered.contains(&block_hash_hex()));
    assert!(rendered.contains("Test Pool"));
    assert!(rendered.contains("Taproot Signaling"));
}

#[test]
fn test_render_templates_and_blocks_filtered_by_pool() {
    let ctx = templates_and_blocks_context(vec![block_fixture()], 1, 0, "Test Pool");
    let rendered = render("templates_and_blocks.html", &ctx);

    assert!(rendered.contains("Recent Templates and Blocks mined by"));
    assert!(rendered.contains(&block_hash_hex()));
}

#[test]
fn test_render_templates_and_blocks_without_blocks() {
    // An empty database: no blocks, no pages and no pools.
    let mut ctx = templates_and_blocks_context(vec![], 0, 0, "");
    ctx.insert("POOLS", &Vec::<String>::new());
    let rendered = render("templates_and_blocks.html", &ctx);

    assert!(rendered.contains("No blocks in the database yet!"));
}

fn block_with_tx_context(block_with_tx: BlockWithTx, sanctioned_missing: usize) -> tera::Context {
    let mut ctx = base_context();
    ctx.insert(
        "THRESHOLD_TRANSACTION_CONSIDERED_YOUNG",
        &THRESHOLD_TRANSACTION_CONSIDERED_YOUNG,
    );
    ctx.insert("TAG_ID_YOUNG", &(tags::TxTag::Young as i32));
    ctx.insert("block_with_tx", &block_with_tx);
    ctx.insert(
        "sanctioned_missing_tx",
        &(0..sanctioned_missing)
            .map(|_| missing_sanctioned_transaction_fixture())
            .collect::<Vec<_>>(),
    );
    ctx
}

fn block_with_tx_fixture() -> BlockWithTx {
    BlockWithTx {
        block: block_fixture(),
        txns_only_in_template: vec![
            (
                transaction_only_in_template_fixture(10, 600),
                transaction_fixture(vec![tags::TxTag::SegWit as i32]),
            ),
            (
                // a young transaction: the template renders an extra tag
                transaction_only_in_template_fixture(11, 10),
                transaction_fixture(vec![]),
            ),
            (
                // an unknown mempool age
                transaction_only_in_template_fixture(12, -1),
                transaction_fixture(vec![tags::TxTag::Taproot as i32]),
            ),
        ],
        txns_only_in_block: vec![(
            transaction_only_in_block_fixture(5),
            transaction_fixture(vec![tags::TxTag::Coinbase as i32]),
        )],
    }
}

#[test]
fn test_render_single_template_and_block() {
    let ctx = block_with_tx_context(block_with_tx_fixture(), 0);
    let rendered = render("subpage/template_and_block.html", &ctx);

    assert_is_a_page(&rendered);
    assert!(rendered.contains(&block_hash_hex()));
    assert!(rendered.contains(&txid_hex()));
    assert!(rendered.contains("Test Pool"));
    assert!(rendered.contains("SegWit spending"));
    assert!(rendered.contains("Young"));
    // the mempool age of -1 is rendered as "Unknown"
    assert!(rendered.contains("Unknown"));
    assert!(!rendered.contains("missing from Block"));
}

#[test]
fn test_render_single_template_and_block_with_sanctioned_transactions() {
    let ctx = block_with_tx_context(block_with_tx_fixture(), 2);
    let rendered = render("subpage/template_and_block.html", &ctx);

    assert!(rendered.contains("Sanctioned Transactions missing from Block"));
    assert!(rendered.contains("1SanctionedAddressxxxxxxxxxxxxxxxx"));
    assert!(rendered.contains("To Sanctioned"));
}

#[test]
fn test_render_single_template_and_block_without_transactions() {
    let block_with_tx = BlockWithTx {
        block: block_fixture(),
        txns_only_in_template: vec![],
        txns_only_in_block: vec![],
    };
    let ctx = block_with_tx_context(block_with_tx, 0);
    let rendered = render("subpage/template_and_block.html", &ctx);

    assert!(rendered.contains(&block_hash_hex()));
}

//##### MISSING SANCTIONED

#[test]
fn test_render_missing_sanctioned() {
    let blocks = vec![block_fixture()];
    let mut ctx = base_context();
    ctx.insert("MAX_BLOCKS_PER_PAGE", &MAX_BLOCKS_PER_PAGE);
    ctx.insert("NAV_PAGE_SANCTIONED", &true);
    ctx.insert("ENTRY_COUNT", &blocks.len());
    ctx.insert("blocks", &blocks);
    let rendered = render("missing-sanctioned.html", &ctx);

    assert_is_a_page(&rendered);
    assert!(rendered.contains(&block_hash_hex()));
    assert!(rendered.contains("Recent Blocks with Missing Sanctioned Transactions (1)"));
}

#[test]
fn test_render_missing_sanctioned_without_blocks() {
    let mut ctx = base_context();
    ctx.insert("MAX_BLOCKS_PER_PAGE", &MAX_BLOCKS_PER_PAGE);
    ctx.insert("NAV_PAGE_SANCTIONED", &true);
    ctx.insert("ENTRY_COUNT", &0);
    ctx.insert(
        "blocks",
        &Vec::<miningpool_observer_shared::model::Block>::new(),
    );
    let rendered = render("missing-sanctioned.html", &ctx);

    assert!(
        rendered.contains("No blocks with missing sanctioned transactions in the database yet!")
    );
}

#[test]
fn test_render_missing_sanctioned_rss() {
    let mut ctx = base_context();
    ctx.insert("blocks_with_missing_sanctioned", &vec![block_fixture()]);
    let rendered = render("rss/sanctioned_missing.xml", &ctx);

    assert!(rendered.contains(&block_hash_hex()));
    assert!(rendered.contains("Test Pool"));
}

#[test]
fn test_render_missing_sanctioned_rss_without_blocks() {
    let mut ctx = base_context();
    ctx.insert(
        "blocks_with_missing_sanctioned",
        &Vec::<miningpool_observer_shared::model::Block>::new(),
    );
    let rendered = render("rss/sanctioned_missing.xml", &ctx);

    assert!(rendered.contains("No Blocks with missing Sanctioned Transactions"));
}

//##### MISSING TRANSACTIONS

fn missing_transaction_fixture(tags: Vec<i32>, heights: &[i32]) -> MissingTransaction {
    MissingTransaction {
        transaction: transaction_fixture(tags),
        blocks: heights
            .iter()
            .map(|h| missing_transaction_block_info_fixture(*h))
            .collect(),
    }
}

fn missing_transactions_context(
    missing_transactions: Vec<MissingTransaction>,
    max_pages: u32,
    current_page: u32,
) -> tera::Context {
    let mut ctx = base_context();
    ctx.insert("NAV_PAGE_MISSING", &true);
    ctx.insert("QUERY_PAGE", &QUERY_PAGE);
    ctx.insert("missing_transactions", &missing_transactions);
    ctx.insert("MAX_PAGES", &max_pages);
    ctx.insert("CURRENT_PAGE", &current_page);
    ctx
}

#[test]
fn test_render_missing_transactions() {
    let missing = vec![missing_transaction_fixture(
        vec![tags::TxTag::SegWit as i32],
        &[700_000, 699_999, 699_998],
    )];
    let rendered = render("missing.html", &missing_transactions_context(missing, 3, 1));

    assert_is_a_page(&rendered);
    assert!(rendered.contains(&txid_hex()));
    assert!(rendered.contains("Test Pool"));
    assert!(rendered.contains("SegWit spending"));
}

#[test]
fn test_render_missing_transactions_without_transactions() {
    let rendered = render("missing.html", &missing_transactions_context(vec![], 0, 0));
    assert!(rendered.contains("No Missing Transactions in the database yet!"));
}

#[test]
fn test_render_missing_transactions_rss() {
    let missing = vec![missing_transaction_fixture(
        vec![tags::TxTag::SegWit as i32],
        &[700_000],
    )];
    let rendered = render(
        "rss/missing.xml",
        &missing_transactions_context(missing, 1, 0),
    );

    assert!(rendered.contains(&txid_hex()));
    assert!(rendered.contains("SegWit spending"));
}

#[test]
fn test_render_missing_transactions_rss_without_transactions() {
    let rendered = render(
        "rss/missing.xml",
        &missing_transactions_context(vec![], 0, 0),
    );
    assert!(rendered.contains("No missing transactions in the database."));
}

#[test]
fn test_render_single_missing_transaction() {
    let mut ctx = base_context();
    ctx.insert(
        "missing_transaction",
        &missing_transaction_fixture(vec![tags::TxTag::Taproot as i32], &[700_000, 699_999]),
    );
    let rendered = render("subpage/missing.html", &ctx);

    assert_is_a_page(&rendered);
    assert!(rendered.contains(&txid_hex()));
    assert!(rendered.contains("Taproot spending"));
    assert!(rendered.contains("Test Pool"));
}

//##### CONFLICTING TRANSACTIONS

#[test]
fn test_render_conflicting_transactions() {
    let mut ctx = base_context();
    ctx.insert("NAV_PAGE_CONFLICTING", &true);
    ctx.insert("MAX_BLOCKS_PER_PAGE", &MAX_BLOCKS_PER_PAGE);
    ctx.insert("QUERY_PAGE", &QUERY_PAGE);
    ctx.insert(
        "blocks_with_conflicting_transactions",
        &vec![conflicting_transaction_info_fixture()],
    );
    ctx.insert("MAX_PAGES", &1u32);
    ctx.insert("CURRENT_PAGE", &0u32);
    let rendered = render("conflicting.html", &ctx);

    assert_is_a_page(&rendered);
    assert!(rendered.contains(&block_hash_hex()));
    assert!(rendered.contains("Test Pool"));
}

#[test]
fn test_render_conflicting_transactions_without_blocks() {
    let mut ctx = base_context();
    ctx.insert("NAV_PAGE_CONFLICTING", &true);
    ctx.insert("MAX_BLOCKS_PER_PAGE", &MAX_BLOCKS_PER_PAGE);
    ctx.insert("QUERY_PAGE", &QUERY_PAGE);
    ctx.insert(
        "blocks_with_conflicting_transactions",
        &Vec::<crate::model::ConflictingTranscationInfo>::new(),
    );
    ctx.insert("MAX_PAGES", &0u32);
    ctx.insert("CURRENT_PAGE", &0u32);
    let rendered = render("conflicting.html", &ctx);

    assert!(rendered.contains("No blocks in the database yet!"));
}

#[test]
fn test_render_single_block_with_conflicting_transactions() {
    let mut ctx = base_context();
    ctx.insert("NAV_PAGE_CONFLICTING", &true);
    ctx.insert("MAX_BLOCKS_PER_PAGE", &MAX_BLOCKS_PER_PAGE);
    ctx.insert(
        "single_block_with_conflicting_transactions",
        &conflicting_transaction_info_fixture(),
    );
    let rendered = render("subpage/conflicting.html", &ctx);

    assert_is_a_page(&rendered);
    assert!(rendered.contains(&block_hash_hex()));
    assert!(rendered.contains(&hex::encode(hash(0xcc))));
    assert!(rendered.contains("Conflicting"));
}

//##### FAQ

#[test]
fn test_render_faq() {
    let tx_tags: Vec<tags::Tag> = tags::TxTag::TX_TAGS.iter().map(|t| t.value()).collect();
    let block_tags: Vec<tags::Tag> = tags::BlockTag::BLOCK_TAGS
        .iter()
        .map(|t| t.value())
        .collect();

    let mut ctx = base_context();
    ctx.insert("TX_TAG_VECTOR", &tx_tags);
    ctx.insert("BLOCK_TAG_VECTOR", &block_tags);
    ctx.insert("NAV_PAGE_FAQ", &true);
    ctx.insert(
        "recent_sanctioned_utxo_scan_info",
        &sanctioned_utxo_scan_info_fixture(),
    );
    ctx.insert(
        "SANCTIONED_ADDRESSES",
        &vec!["1SanctionedAddressxxxxxxxxxxxxxxxx".to_string()],
    );
    let rendered = render("faq.html", &ctx);

    assert_is_a_page(&rendered);
    assert!(rendered.contains("Coinbase"));
    assert!(rendered.contains("Taproot Signaling"));
    assert!(rendered.contains("1SanctionedAddressxxxxxxxxxxxxxxxx"));
    assert!(rendered.contains("This site currently tracks 1 OFAC sanctioned addresses."));
}

//##### DEBUG PAGES

#[test]
fn test_render_debug_index() {
    let rendered = render("debug/index.html", &base_context());
    assert_is_a_page(&rendered);
}

#[test]
fn test_render_debug_utxo_set_scans() {
    let mut ctx = base_context();
    ctx.insert("MAX_BLOCKS_PER_PAGE", &MAX_BLOCKS_PER_PAGE);
    ctx.insert("QUERY_PAGE", &QUERY_PAGE);
    ctx.insert("scans", &vec![sanctioned_utxo_scan_info_fixture()]);
    ctx.insert("MAX_PAGES", &1u32);
    ctx.insert("CURRENT_PAGE", &0u32);
    let rendered = render("debug/utxo_set_scans.html", &ctx);

    assert_is_a_page(&rendered);
    assert!(rendered.contains("42 seconds"));
    // the page size is not the block page size
    assert!(MAX_UTXOSET_SCANS_PER_PAGE > MAX_BLOCKS_PER_PAGE);
}

#[test]
fn test_render_debug_unknown_pool_blocks() {
    let mut ctx = base_context();
    ctx.insert("unknown_pool_blocks", &vec![block_fixture()]);
    let rendered = render("debug/unknown_pool_blocks.html", &ctx);

    assert_is_a_page(&rendered);
    assert!(rendered.contains(&block_hash_hex()));
}

#[test]
fn test_render_debug_fees_by_pool() {
    let avg_fees = vec![AvgPoolFees {
        pool_name: "Test Pool".to_string(),
        count: 11,
        median: 0.99,
        q1: 0.98,
        q3: 1.0,
    }];
    let mut ctx = base_context();
    ctx.insert("avgfees", &avg_fees);
    let rendered = render("debug/fees_by_pool.html", &ctx);

    assert_is_a_page(&rendered);
    assert!(rendered.contains("Test Pool"));
}

#[test]
fn test_render_debug_template_selection() {
    let infos_and_block = DebugTemplateSelectionInfosAndBlock {
        block: block_fixture(),
        infos: vec![
            debug_template_selection_info_fixture(true),
            debug_template_selection_info_fixture(false),
        ],
    };
    let mut ctx = base_context();
    ctx.insert("MAX_BLOCKS_PER_PAGE", &MAX_BLOCKS_PER_PAGE);
    ctx.insert("QUERY_PAGE", &QUERY_PAGE);
    ctx.insert("template_selection_infos", &vec![infos_and_block]);
    ctx.insert("MAX_PAGES", &1u32);
    ctx.insert("CURRENT_PAGE", &0u32);
    let rendered = render("debug/template_selection.html", &ctx);

    assert_is_a_page(&rendered);
    assert!(rendered.contains(&block_hash_hex()));
}

#[test]
fn test_render_debug_sanctioned_by_pool() {
    let table = vec![PoolSanctionedTableEntry {
        pool_name: "Test Pool".to_string(),
        in_both: 5,
        only_in_block: 1,
        only_in_template: 2,
    }];
    let mut ctx = base_context();
    ctx.insert("sanctioned_table", &table);
    let rendered = render("debug/sanctioned_by_pool.html", &ctx);

    assert_is_a_page(&rendered);
    assert!(rendered.contains("Test Pool"));
}

#[test]
fn test_render_debug_sanctioned_rss() {
    let mut ctx = base_context();
    ctx.insert(
        "templates_and_blocks_with_sanctioned_tx",
        &vec![block_fixture()],
    );
    let rendered = render("debug/rss/sanctioned.rss", &ctx);

    assert!(rendered.contains(&block_hash_hex()));
    // the fixture has two sanctioned transactions in the template and one in
    // the block
    assert!(rendered.contains("The template includes 2 sanctioned transactions"));
}

#[test]
fn test_render_debug_sanctioned_rss_only_in_template() {
    let mut block = block_fixture();
    block.template_sanctioned = 1;
    block.block_sanctioned = 0;

    let mut ctx = base_context();
    ctx.insert("templates_and_blocks_with_sanctioned_tx", &vec![block]);
    let rendered = render("debug/rss/sanctioned.rss", &ctx);

    assert!(rendered.contains("Test Pool"));
    assert!(rendered.contains("might filter transactions"));
}

#[test]
fn test_render_debug_sanctioned_rss_without_blocks() {
    let mut ctx = base_context();
    ctx.insert(
        "templates_and_blocks_with_sanctioned_tx",
        &Vec::<miningpool_observer_shared::model::Block>::new(),
    );
    let rendered = render("debug/rss/sanctioned.rss", &ctx);

    assert!(rendered.contains("No Blocks with Sanctioned Transactions"));
}

//##### OPENGRAPH IMAGES

fn assert_renders_as_png(svg: &str) {
    let png = render_and_encode(svg, &usvg_options()).expect("the SVG should render");
    assert_eq!(&png[..8], &PNG_SIGNATURE);
}

fn svg_context() -> tera::Context {
    let mut ctx = tera::Context::new();
    ctx.insert("config", &site_config());
    ctx
}

#[test]
fn test_render_mainpage_images() {
    for template in [
        "svg/mainpage_index.svg",
        "svg/mainpage_faq.svg",
        "svg/mainpage_templates_and_blocks.svg",
        "svg/mainpage_missing_transactions.svg",
        "svg/mainpage_conflicting_transactions.svg",
        "svg/mainpage_sanctioned_transactions.svg",
    ]
    .iter()
    {
        let rendered = render(template, &svg_context());
        assert!(rendered.contains("Test Observer"));
        assert_renders_as_png(&rendered);
    }
}

#[test]
fn test_render_template_and_block_image() {
    // mirrors the Data struct in ogimage::ogimage_template_and_block
    #[derive(Serialize)]
    struct Data {
        hash: String,
        height: i32,
        pool: String,
        missing: i32,
        extra: i32,
        shared: i32,
    }

    let block = block_fixture();
    let mut ctx = svg_context();
    ctx.insert(
        "data",
        &Data {
            hash: block_hash_hex(),
            height: block.height,
            pool: block.pool_name,
            missing: block.missing_tx,
            extra: block.extra_tx,
            shared: block.shared_tx,
        },
    );
    let rendered = render("svg/subpage_template_and_block.svg", &ctx);

    assert!(rendered.contains("Test Pool"));
    assert_renders_as_png(&rendered);
}

#[test]
fn test_render_block_with_conflicting_transactions_image() {
    // mirrors the Data struct in
    // ogimage::ogimage_block_with_conflicting_transactions
    #[derive(Serialize)]
    struct Data {
        hash: String,
        pool: String,
        height: i32,
        conflicts: usize,
    }

    let mut ctx = svg_context();
    ctx.insert(
        "data",
        &Data {
            hash: block_hash_hex(),
            pool: "Test Pool".to_string(),
            height: 700_000,
            conflicts: 2,
        },
    );
    let rendered = render("svg/subpage_block_with_conflicting_transactions.svg", &ctx);

    assert!(rendered.contains("Test Pool"));
    assert_renders_as_png(&rendered);
}

#[test]
fn test_render_block_with_sanctioned_transactions_image() {
    #[derive(Serialize)]
    struct Data {
        hash: String,
        pool: String,
        both: i32,
        template: i32,
        block: i32,
    }

    let mut ctx = svg_context();
    ctx.insert(
        "data",
        &Data {
            hash: block_hash_hex(),
            pool: "Test Pool".to_string(),
            both: 1,
            template: 2,
            block: 1,
        },
    );
    let rendered = render("svg/subpage_block_with_sanctioned_transactions.svg", &ctx);

    assert!(rendered.contains("Test Pool"));
    assert_renders_as_png(&rendered);
}

#[test]
fn test_render_missing_transaction_image() {
    // mirrors MissingTransactionData in ogimage
    #[derive(Serialize)]
    struct Data {
        block_count: usize,
        txid: String,
        feerate: f32,
        size: i32,
        fee: i64,
        tags: Vec<String>,
    }

    let mut ctx = svg_context();
    ctx.insert(
        "data",
        &Data {
            block_count: 3,
            txid: txid_hex(),
            feerate: 3.0,
            size: 300,
            fee: 1000,
            tags: vec!["<tspan fill=\"#00a6d6\">Young</tspan>".to_string()],
        },
    );
    let rendered = render("svg/subpage_missing_transaction.svg", &ctx);

    assert!(rendered.contains(&txid_hex()));
    assert_renders_as_png(&rendered);
}
