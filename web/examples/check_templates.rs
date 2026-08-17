use miningpool_observer_shared::{config, tags};
use serde_json::json;
use std::convert::TryFrom;

fn tx_tag_id_to_tag(kwargs: tera::Kwargs, _: &tera::State) -> tera::TeraResult<tera::Value> {
    let id: i32 = kwargs.must_get("id")?;
    let tag = tags::TxTag::try_from(id).map_err(|_| tera::Error::message("bad tx tag id"))?;
    Ok(tera::Value::from_serializable(&tag.value()))
}

fn block_tag_id_to_tag(kwargs: tera::Kwargs, _: &tera::State) -> tera::TeraResult<tera::Value> {
    let id: i32 = kwargs.must_get("id")?;
    let tag = tags::BlockTag::try_from(id).map_err(|_| tera::Error::message("bad block tag id"))?;
    Ok(tera::Value::from_serializable(&tag.value()))
}

fn seconds_to_duration(kwargs: tera::Kwargs, _: &tera::State) -> tera::TeraResult<String> {
    let seconds: i32 = kwargs.must_get("seconds")?;
    Ok(format!("{}s", seconds))
}

fn block_fixture(hash: &str, pool_name: &str) -> serde_json::Value {
    json!({
        "id": 1,
        "hash": hash,
        "prev_hash": "00".repeat(32),
        "height": 800000,
        "tags": [3100],
        "missing_tx": 3,
        "extra_tx": 2,
        "shared_tx": 1000,
        "sanctioned_missing_tx": 1,
        "equality": 0.95,
        "block_time": 1700000000,
        "block_seen_time": 1700000005,
        "block_tx": 1500,
        "block_sanctioned": 0,
        "block_cb_value": 650000000i64,
        "block_cb_fees": 25000000i64,
        "block_weight": 3990000,
        "block_pkg_weights": [4000, 8000],
        "block_pkg_feerates": [12.5, 8.2],
        "pool_name": pool_name,
        "pool_link": "https://example.com",
        "pool_id_method": "coinbase_tag",
        "template_tx": 1502,
        "template_time": 1699999995,
        "template_sanctioned": 1,
        "template_cb_value": 650100000i64,
        "template_cb_fees": 25100000i64,
        "template_weight": 3995000,
        "template_pkg_weights": [4000, 8000],
        "template_pkg_feerates": [12.6, 8.3],
        "template_sigops": 12000,
        "block_sigops": 11800,
        "prev_hash_display": "prev",
    })
}

fn transaction_fixture(txid: &str) -> serde_json::Value {
    json!({
        "txid": txid,
        "sanctioned": false,
        "vsize": 250,
        "fee": 5000,
        "output_sum": 1000000i64,
        "tags": [3110],
        "input_count": 1,
        "inputs": ["input 1"],
        "output_count": 2,
        "outputs": ["output 1", "output 2"],
        "sigops": 4,
    })
}

fn main() {
    let mut tera = tera::Tera::new();
    tera.register_function("block_tag_id_to_tag", block_tag_id_to_tag);
    tera.register_function("tx_tag_id_to_tag", tx_tag_id_to_tag);
    tera.register_function("seconds_to_duration", seconds_to_duration);
    tera.register_function("now", tera_contrib::dates::now);
    tera.register_filter("date", tera_contrib::dates::date);
    tera.register_filter("urlencode", tera_contrib::urlencode::urlencode);

    if let Err(e) = tera.load_from_glob("www/templates/**/*") {
        eprintln!("ERROR loading templates: {}", e);
        std::process::exit(1);
    }
    println!("templates loaded OK");

    let site_config = config::WebSiteConfig {
        title: "Test Site".to_string(),
        footer: "footer".to_string(),
        base_url: "https://example.test".to_string(),
    };

    let hash = "0".repeat(64);
    let txid = "1".repeat(64);

    let block = block_fixture(&hash, "TestPool");
    let transaction = transaction_fixture(&txid);

    let missing_transaction = json!({
        "transaction": transaction,
        "blocks": [{
            "hash": hash,
            "time": 1700000000,
            "height": 800000,
            "pool": "TestPool",
            "template_position": 3,
            "mempool_age": 120,
            "template_tx_count": 1500,
            "last_block_pkg_feerate": 10.5,
        }],
    });

    let block_with_tx = json!({
        "block": block,
        "txns_only_in_template": [[
            {"block_id": 1, "position": 0, "mempool_age_seconds": 30, "transaction_txid": txid},
            transaction,
        ]],
        "txns_only_in_block": [[
            {"block_id": 1, "position": 0, "transaction_txid": txid},
            transaction,
        ]],
    });

    let sanctioned_missing_tx = json!([{
        "transaction": transaction,
        "missing_info": {"block_id": 1, "position": 0, "mempool_age_seconds": 30, "transaction_txid": txid},
        "addresses": ["addr1"],
    }]);

    let conflicting_transaction_sets = json!([{
        "template_transactions": [transaction],
        "block_transactions": [transaction],
        "conflicting_outpoints": [{"txid": txid, "vout": 0}],
    }]);

    let conflicting_info = json!({
        "block": block,
        "conflicting_transaction_sets": conflicting_transaction_sets,
    });

    let debug_template_selection_infos_and_block = json!([{
        "block": block,
        "infos": [{
            "block_id": 1,
            "template_time": 1699999995,
            "count_missing": 3,
            "count_shared": 1000,
            "count_extra": 2,
            "selected": true,
        }],
    }]);

    let tx_tags: Vec<tags::Tag> = tags::TxTag::TX_TAGS.iter().map(|t| t.value()).collect();
    let block_tags: Vec<tags::Tag> = tags::BlockTag::BLOCK_TAGS
        .iter()
        .map(|t| t.value())
        .collect();

    let mut failures = Vec::new();
    let mut check = |name: &str, ctx: &tera::Context| match tera.render(name, ctx) {
        Ok(s) => {
            println!("OK   {} ({} bytes)", name, s.len());
        }
        Err(e) => {
            println!("FAIL {}: {}", name, e);
            let mut source = std::error::Error::source(&e);
            while let Some(s) = source {
                println!("       caused by: {}", s);
                source = std::error::Error::source(s);
            }
            failures.push(name.to_string());
        }
    };

    // index.html
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    check("index.html", &ctx);

    // error.html
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert("status_code", &404);
    ctx.insert("error", "Not Found");
    check("error.html", &ctx);

    // templates_and_blocks.html
    let mut ctx = tera::Context::new();
    ctx.insert("MAX_BLOCKS_PER_PAGE", &20u32);
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert("NAV_PAGE_BLOCKS", &true);
    ctx.insert("QUERY_PAGE", "page");
    ctx.insert("QUERY_POOL", "pool");
    ctx.insert("blocks", &json!([block]));
    ctx.insert("MAX_PAGES", &5u32);
    ctx.insert("CURRENT_PAGE", &0u32);
    ctx.insert("CURRENT_POOL", "");
    ctx.insert("POOLS", &json!(["TestPool", "Unknown"]));
    check("templates_and_blocks.html", &ctx);

    // subpage/template_and_block.html
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert(
        "THRESHOLD_TRANSACTION_CONSIDERED_YOUNG",
        &tags::THRESHOLD_TRANSACTION_CONSIDERED_YOUNG,
    );
    ctx.insert("TAG_ID_YOUNG", &(tags::TxTag::Young as i32));
    ctx.insert("block_with_tx", &block_with_tx);
    ctx.insert("sanctioned_missing_tx", &sanctioned_missing_tx);
    check("subpage/template_and_block.html", &ctx);

    // missing-sanctioned.html
    let mut ctx = tera::Context::new();
    ctx.insert("MAX_BLOCKS_PER_PAGE", &20u32);
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert("NAV_PAGE_SANCTIONED", &true);
    ctx.insert("blocks", &json!([block]));
    ctx.insert("ENTRY_COUNT", &1);
    check("missing-sanctioned.html", &ctx);

    // missing.html
    let mut ctx = tera::Context::new();
    ctx.insert("NAV_PAGE_MISSING", &true);
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert("QUERY_PAGE", "page");
    ctx.insert("missing_transactions", &json!([missing_transaction]));
    ctx.insert("MAX_PAGES", &5u32);
    ctx.insert("CURRENT_PAGE", &0u32);
    check("missing.html", &ctx);

    // subpage/missing.html
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert("missing_transaction", &missing_transaction);
    check("subpage/missing.html", &ctx);

    // rss/missing.xml
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert("missing_transactions", &json!([missing_transaction]));
    ctx.insert("MAX_PAGES", &5u32);
    ctx.insert("CURRENT_PAGE", &0u32);
    check("rss/missing.xml", &ctx);

    // conflicting.html
    let mut ctx = tera::Context::new();
    ctx.insert("NAV_PAGE_CONFLICTING", &true);
    ctx.insert("MAX_BLOCKS_PER_PAGE", &20u32);
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert("QUERY_PAGE", "page");
    ctx.insert(
        "blocks_with_conflicting_transactions",
        &json!([conflicting_info]),
    );
    ctx.insert("MAX_PAGES", &5u32);
    ctx.insert("CURRENT_PAGE", &0u32);
    check("conflicting.html", &ctx);

    // subpage/conflicting.html
    let mut ctx = tera::Context::new();
    ctx.insert("NAV_PAGE_CONFLICTING", &true);
    ctx.insert("MAX_BLOCKS_PER_PAGE", &20u32);
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert(
        "single_block_with_conflicting_transactions",
        &conflicting_info,
    );
    check("subpage/conflicting.html", &ctx);

    // faq.html
    let mut ctx = tera::Context::new();
    ctx.insert("TX_TAG_VECTOR", &tx_tags);
    ctx.insert("BLOCK_TAG_VECTOR", &block_tags);
    ctx.insert("NAV_PAGE_FAQ", &true);
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert(
        "recent_sanctioned_utxo_scan_info",
        &json!({
            "end_time": 1700000000,
            "end_height": 800000,
            "duration_seconds": 30,
            "utxo_amount": 123456789,
            "utxo_count": 42,
        }),
    );
    ctx.insert("SANCTIONED_ADDRESSES", &json!(["addr1", "addr2"]));
    check("faq.html", &ctx);

    // debug/index.html
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    check("debug/index.html", &ctx);

    // debug/utxo_set_scans.html
    let mut ctx = tera::Context::new();
    ctx.insert("MAX_BLOCKS_PER_PAGE", &20u32);
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert("QUERY_PAGE", "page");
    ctx.insert(
        "scans",
        &json!([{
            "end_time": 1700000000,
            "end_height": 800000,
            "duration_seconds": 30,
            "utxo_amount": 123456789,
            "utxo_count": 42,
        }]),
    );
    ctx.insert("MAX_PAGES", &5u32);
    ctx.insert("CURRENT_PAGE", &0u32);
    check("debug/utxo_set_scans.html", &ctx);

    // debug/unknown_pool_blocks.html
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert("unknown_pool_blocks", &json!([block]));
    check("debug/unknown_pool_blocks.html", &ctx);

    // debug/fees_by_pool.html
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert(
        "avgfees",
        &json!([{"pool_name": "TestPool", "count": 10, "median": 1.0, "q1": 0.5, "q3": 1.5}]),
    );
    check("debug/fees_by_pool.html", &ctx);

    // debug/template_selection.html
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert("MAX_BLOCKS_PER_PAGE", &20u32);
    ctx.insert("QUERY_PAGE", "page");
    ctx.insert(
        "template_selection_infos",
        &debug_template_selection_infos_and_block,
    );
    ctx.insert("MAX_PAGES", &5u32);
    ctx.insert("CURRENT_PAGE", &0u32);
    check("debug/template_selection.html", &ctx);

    // debug/sanctioned_by_pool.html
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert(
        "sanctioned_table",
        &json!([{"pool_name": "TestPool", "in_both": 1, "only_in_block": 2, "only_in_template": 3}]),
    );
    check("debug/sanctioned_by_pool.html", &ctx);

    // debug/rss/sanctioned.rss
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert("templates_and_blocks_with_sanctioned_tx", &json!([block]));
    check("debug/rss/sanctioned.rss", &ctx);

    // rss/sanctioned_missing.xml
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", &site_config);
    ctx.insert("NODE_VERSION", "1.0.0");
    ctx.insert("blocks_with_missing_sanctioned", &json!([block]));
    check("rss/sanctioned_missing.xml", &ctx);

    // svg og-image templates
    for svg in [
        "svg/mainpage_index.svg",
        "svg/mainpage_templates_and_blocks.svg",
        "svg/mainpage_missing_transactions.svg",
        "svg/mainpage_conflicting_transactions.svg",
        "svg/mainpage_sanctioned_transactions.svg",
        "svg/mainpage_faq.svg",
    ] {
        let mut ctx = tera::Context::new();
        ctx.insert("config", &site_config);
        check(svg, &ctx);
    }

    let mut ctx = tera::Context::new();
    ctx.insert("config", &site_config);
    ctx.insert(
        "data",
        &json!({"block_count": 3, "txid": txid, "feerate": 20.0, "size": 250, "fee": 5000, "tags": ["Young"]}),
    );
    check("svg/subpage_missing_transaction.svg", &ctx);

    let mut ctx = tera::Context::new();
    ctx.insert("config", &site_config);
    ctx.insert(
        "data",
        &json!({"hash": hash, "height": 800000, "pool": "TestPool", "missing": 3, "extra": 2, "shared": 1000}),
    );
    check("svg/subpage_template_and_block.svg", &ctx);

    let mut ctx = tera::Context::new();
    ctx.insert("config", &site_config);
    ctx.insert(
        "data",
        &json!({"hash": hash, "height": 800000, "pool": "TestPool", "conflicts": 3}),
    );
    check("svg/subpage_block_with_conflicting_transactions.svg", &ctx);

    if !failures.is_empty() {
        eprintln!(
            "\n{} template(s) failed to render: {:?}",
            failures.len(),
            failures
        );
        std::process::exit(1);
    }
    println!("\nAll templates rendered successfully.");
}
