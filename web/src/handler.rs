use actix_web::{web, Error, HttpResponse, Result};

use crate::{db, error, model, util};

use db::MAX_BLOCKS_PER_PAGE;
use miningpool_observer_shared::{config, db_pool, tags};
use tags::THRESHOLD_TRANSACTION_CONSIDERED_YOUNG;

use std::collections::HashMap;

const QUERY_PAGE: &str = "page";
const QUERY_POOL: &str = "pool";

//##### INDEX

pub async fn index(
    tmpl: web::Data<tera::Tera>,
    node_version: web::Data<String>,
    config: web::Data<config::WebSiteConfig>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());
    let s = tmpl
        .render("index.html", &ctx)
        .map_err(error::template_error)?;
    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

//##### TEMPLATE & BLOCK

pub async fn templates_and_blocks(
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
    query: web::Query<HashMap<String, String>>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    ctx.insert("MAX_BLOCKS_PER_PAGE", &MAX_BLOCKS_PER_PAGE);
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());
    ctx.insert("NAV_PAGE_BLOCKS", &true);
    ctx.insert("QUERY_PAGE", &QUERY_PAGE);
    ctx.insert("QUERY_POOL", &QUERY_POOL);

    let mut page = 0u32;
    if let Some(query_page) = query.get(QUERY_PAGE) {
        page = util::parse_uint(query_page)?;
    }

    let mut mining_pool = String::default();
    if let Some(query_pool) = query.get(QUERY_POOL) {
        mining_pool = query_pool.to_string();
    }

    let conn = pool.get().expect("couldn't get db connection from pool");
    if mining_pool != String::default() {
        let moved_mining_pool = mining_pool.clone();
        let (blocks, max_pages) =
            web::block(move || db::blocks_by_pool(&conn, page, &moved_mining_pool))
                .await
                .map_err(error::database_error)?;
        ctx.insert("blocks", &blocks);
        ctx.insert("MAX_PAGES", &max_pages);
    } else {
        let (blocks, max_pages) = web::block(move || db::blocks(&conn, page))
            .await
            .map_err(error::database_error)?;
        ctx.insert("blocks", &blocks);
        ctx.insert("MAX_PAGES", &max_pages);
    }
    ctx.insert("CURRENT_PAGE", &page);
    ctx.insert("CURRENT_POOL", &mining_pool);

    let conn = pool.get().expect("couldn't get db connection from pool");
    let pools = web::block(move || db::pools(&conn))
        .await
        .map_err(error::database_error)?;

    ctx.insert("POOLS", &pools);

    let s = tmpl
        .render("templates_and_blocks.html", &ctx)
        .map_err(error::template_error)?;
    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

pub async fn single_template_and_block(
    hash_str: web::Path<String>,
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
) -> Result<HttpResponse, Error> {
    let hash = util::parse_block_hash_str(&hash_str)?;

    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());
    ctx.insert(
        "THRESHOLD_TRANSACTION_CONSIDERED_YOUNG",
        &THRESHOLD_TRANSACTION_CONSIDERED_YOUNG,
    );
    ctx.insert("TAG_ID_YOUNG", &(tags::TxTag::Young as i32));

    let hash_clone = hash.clone();
    let conn = pool.get().expect("couldn't get db connection from pool");
    let block_with_tx = web::block(move || db::block_with_tx(&hash, &conn))
        .await
        .map_err(error::database_error)?;
    ctx.insert("block_with_tx", &block_with_tx);

    if block_with_tx.block.sanctioned_missing_tx > 0 {
        let conn = pool.get().expect("couldn't get db connection from pool");
        let sanctioned_missing_tx =
            web::block(move || db::missing_sanctioned_txns_for_block(&hash_clone, &conn))
                .await
                .map_err(error::database_error)?;
        ctx.insert("sanctioned_missing_tx", &sanctioned_missing_tx);
    } else {
        let empty: &[model::MissingSanctionedTransaction] = &[];
        ctx.insert("sanctioned_missing_tx", empty);
    }

    let s = tmpl
        .render("subpage/template_and_block.html", &ctx)
        .map_err(error::template_error)?;

    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

pub async fn missing_sanctioned_transactions_rss(
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());

    let conn = pool.get().expect("couldn't get db connection from pool");

    let blocks_with_missing_sanctioned =
        web::block(move || db::blocks_with_missing_sanctioned(&conn))
            .await
            .map_err(error::database_error)?;
    ctx.insert(
        "blocks_with_missing_sanctioned",
        &blocks_with_missing_sanctioned,
    );

    let s = tmpl
        .render("rss/sanctioned_missing.xml", &ctx)
        .map_err(error::template_error)?;
    Ok(HttpResponse::Ok()
        .content_type("application/rss+xml")
        .body(s))
}

//##### MISSING TRANSCTIONS

pub async fn missing_transactions(
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
    query: web::Query<HashMap<String, String>>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    ctx.insert("NAV_PAGE_MISSING", &true);
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());
    ctx.insert("QUERY_PAGE", &QUERY_PAGE);

    let mut page = 0u32;
    if let Some(query_page) = query.get(QUERY_PAGE) {
        page = util::parse_uint(query_page)?;
    }

    let conn = pool.get().expect("couldn't get db connection from pool");
    let (missing_transactions, max_pages) =
        web::block(move || db::missing_transactions(&conn, page))
            .await
            .map_err(error::database_error)?;
    ctx.insert("missing_transactions", &missing_transactions);
    ctx.insert("MAX_PAGES", &max_pages);
    ctx.insert("CURRENT_PAGE", &page);

    let s = tmpl
        .render("missing.html", &ctx)
        .map_err(error::template_error)?;

    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

pub async fn single_missing_transaction(
    txid_str: web::Path<String>,
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
) -> Result<HttpResponse, Error> {
    let txid = util::parse_txid_str(&txid_str)?;
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());
    let conn = pool.get().expect("couldn't get db connection from pool");
    let missing_transaction = web::block(move || db::single_missing_transaction(&txid, &conn))
        .await
        .map_err(error::database_error)?;

    ctx.insert("missing_transaction", &missing_transaction);
    let s = tmpl
        .render("subpage/missing.html", &ctx)
        .map_err(error::template_error)?;

    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

pub async fn missing_transactions_rss(
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
    query: web::Query<HashMap<String, String>>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());
    ctx.insert("QUERY_PAGE", &QUERY_PAGE);

    let mut page = 0u32;
    if let Some(query_page) = query.get(QUERY_PAGE) {
        page = util::parse_uint(query_page)?;
    }

    let conn = pool.get().expect("couldn't get db connection from pool");
    let (missing_transactions, max_pages) =
        web::block(move || db::missing_transactions(&conn, page))
            .await
            .map_err(error::database_error)?;
    ctx.insert("missing_transactions", &missing_transactions);
    ctx.insert("MAX_PAGES", &max_pages);
    ctx.insert("CURRENT_PAGE", &page);

    let s = tmpl
        .render("rss/missing.xml", &ctx)
        .map_err(error::template_error)?;

    Ok(HttpResponse::Ok()
        .content_type("application/rss+xml")
        .body(s))
}

//##### CONFLICTING TRANSCTIONS

pub async fn conflicting_transactions(
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
    query: web::Query<HashMap<String, String>>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    ctx.insert("NAV_PAGE_CONFLICTING", &true);
    ctx.insert("MAX_BLOCKS_PER_PAGE", &MAX_BLOCKS_PER_PAGE);
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());
    ctx.insert("QUERY_PAGE", &QUERY_PAGE);

    let mut page = 0u32;
    if let Some(query_page) = query.get(QUERY_PAGE) {
        page = util::parse_uint(query_page)?;
    }

    let conn = pool.get().expect("couldn't get db connection from pool");
    let (blocks_with_conflicting_transctions, max_pages) =
        web::block(move || db::blocks_with_conflicting_transactions(&conn, page))
            .await
            .map_err(error::database_error)?;

    ctx.insert(
        "blocks_with_conflicting_transactions",
        &blocks_with_conflicting_transctions,
    );
    ctx.insert("MAX_PAGES", &max_pages);
    ctx.insert("CURRENT_PAGE", &page);

    let s = tmpl
        .render("conflicting.html", &ctx)
        .map_err(error::template_error)?;
    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

pub async fn single_block_with_conflicting_transactions(
    hash_str: web::Path<String>,
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
) -> Result<HttpResponse, Error> {
    let hash = util::parse_block_hash_str(&hash_str)?;

    let mut ctx = tera::Context::new();
    ctx.insert("NAV_PAGE_CONFLICTING", &true);
    ctx.insert("MAX_BLOCKS_PER_PAGE", &MAX_BLOCKS_PER_PAGE);
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());

    let conn = pool.get().expect("couldn't get db connection from pool");
    let single_block_with_conflicting_transactions =
        web::block(move || db::single_block_with_conflicting_transactions(&conn, &hash))
            .await
            .map_err(error::database_error)?;

    ctx.insert(
        "single_block_with_conflicting_transactions",
        &single_block_with_conflicting_transactions,
    );

    let s = tmpl
        .render("subpage/conflicting.html", &ctx)
        .map_err(error::template_error)?;

    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

//##### OTHER PAGES

pub async fn robots_txt() -> Result<HttpResponse, Error> {
    let robots_txt = "User-agent: *
Allow: /
Disallow: /debug/*";
    Ok(HttpResponse::Ok()
        .content_type("text/plain")
        .body(robots_txt))
}

include!(concat!(env!("OUT_DIR"), "/list_sanctioned_addr.rs"));

pub async fn faq(
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    let tx_tags: Vec<tags::Tag> = tags::TxTag::TX_TAGS.iter().map(|t| t.value()).collect();
    let block_tags: Vec<tags::Tag> = tags::BlockTag::BLOCK_TAGS
        .iter()
        .map(|t| t.value())
        .collect();
    ctx.insert("TX_TAG_VECTOR", &tx_tags);
    ctx.insert("BLOCK_TAG_VECTOR", &block_tags);
    ctx.insert("NAV_PAGE_FAQ", &true);
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());
    ctx.insert("SANCTIONED_ADDRESSES", &get_sanctioned_addresses());

    let conn = pool.get().expect("couldn't get db connection from pool");
    let recent_sanctioned_utxo_scan_info =
        web::block(move || db::get_recent_sanctioned_utxo_scan_info(&conn))
            .await
            .map_err(error::database_error)?;
    ctx.insert(
        "recent_sanctioned_utxo_scan_info",
        &recent_sanctioned_utxo_scan_info,
    );

    let s = tmpl
        .render("faq.html", &ctx)
        .map_err(error::template_error)?;
    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

//##### DEBUG PAGES

pub async fn debug(
    tmpl: web::Data<tera::Tera>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
    debug_pages_enabled: web::Data<bool>,
) -> Result<HttpResponse, Error> {
    if !debug_pages_enabled.get_ref() {
        return Ok(HttpResponse::NotFound().finish());
    }
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());
    let s = tmpl
        .render("debug/index.html", &ctx)
        .map_err(error::template_error)?;
    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

pub async fn debug_utxoset_scans(
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
    query: web::Query<HashMap<String, String>>,
    debug_pages_enabled: web::Data<bool>,
) -> Result<HttpResponse, Error> {
    if !debug_pages_enabled.get_ref() {
        return Ok(HttpResponse::NotFound().finish());
    }

    let mut page = 0u32;
    if let Some(query_page) = query.get(QUERY_PAGE) {
        page = util::parse_uint(query_page)?;
    }

    let conn = pool.get().expect("couldn't get db connection from pool");
    let mut ctx = tera::Context::new();
    ctx.insert("MAX_BLOCKS_PER_PAGE", &MAX_BLOCKS_PER_PAGE);
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());
    ctx.insert("QUERY_PAGE", &QUERY_PAGE);

    let (scans, max_pages) = web::block(move || db::sanctioned_utxo_scan_infos(&conn, page))
        .await
        .map_err(error::database_error)?;

    ctx.insert("scans", &scans);
    ctx.insert("MAX_PAGES", &max_pages);
    ctx.insert("CURRENT_PAGE", &page);

    let s = tmpl
        .render("debug/utxo_set_scans.html", &ctx)
        .map_err(error::template_error)?;
    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

pub async fn debug_unknown_pool_blocks(
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
    debug_pages_enabled: web::Data<bool>,
) -> Result<HttpResponse, Error> {
    if !debug_pages_enabled.get_ref() {
        return Ok(HttpResponse::NotFound().finish());
    }

    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());

    let conn = pool.get().expect("couldn't get db connection from pool");
    let unknown_pool_blocks = web::block(move || db::unknown_pool_blocks(&conn))
        .await
        .map_err(error::database_error)?;

    ctx.insert("unknown_pool_blocks", &unknown_pool_blocks);
    let s = tmpl
        .render("debug/unknown_pool_blocks.html", &ctx)
        .map_err(error::template_error)?;

    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

pub async fn debug_fees_by_pool(
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
    debug_pages_enabled: web::Data<bool>,
) -> Result<HttpResponse, Error> {
    if !debug_pages_enabled.get_ref() {
        return Ok(HttpResponse::NotFound().finish());
    }

    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());

    let conn = pool.get().expect("couldn't get db connection from pool");
    let avg_fees = web::block(move || db::avg_fees_by_pool(&conn))
        .await
        .map_err(error::database_error)?;

    ctx.insert("avgfees", &avg_fees);
    let s = tmpl
        .render("debug/fees_by_pool.html", &ctx)
        .map_err(error::template_error)?;

    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

pub async fn debug_template_selection_infos(
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
    query: web::Query<HashMap<String, String>>,
    debug_pages_enabled: web::Data<bool>,
) -> Result<HttpResponse, Error> {
    if !debug_pages_enabled.get_ref() {
        return Ok(HttpResponse::NotFound().finish());
    }

    let mut page = 0u32;
    if let Some(query_page) = query.get(QUERY_PAGE) {
        page = util::parse_uint(query_page)?;
    }

    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());
    ctx.insert("MAX_BLOCKS_PER_PAGE", &MAX_BLOCKS_PER_PAGE);
    ctx.insert("QUERY_PAGE", &QUERY_PAGE);

    let conn = pool.get().expect("couldn't get db connection from pool");
    let (template_selection_infos, max_pages) =
        web::block(move || db::debug_template_selection_infos(&conn, page))
            .await
            .map_err(error::database_error)?;

    ctx.insert("template_selection_infos", &template_selection_infos);
    ctx.insert("MAX_PAGES", &max_pages);
    ctx.insert("CURRENT_PAGE", &page);
    let s = tmpl
        .render("debug/template_selection.html", &ctx)
        .map_err(error::template_error)?;

    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

pub async fn debug_sanctioned_by_pool(
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
    debug_pages_enabled: web::Data<bool>,
) -> Result<HttpResponse, Error> {
    if !debug_pages_enabled.get_ref() {
        return Ok(HttpResponse::NotFound().finish());
    }

    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());

    let conn = pool.get().expect("couldn't get db connection from pool");
    let sanctioned_table = web::block(move || db::debug_sanctioned_table(&conn))
        .await
        .map_err(error::database_error)?;

    ctx.insert("sanctioned_table", &sanctioned_table);
    let s = tmpl
        .render("debug/sanctioned_by_pool.html", &ctx)
        .map_err(error::template_error)?;

    Ok(HttpResponse::Ok().content_type("text/html").body(s))
}

pub async fn debug_sanctioned_transactions_rss(
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    node_version: web::Data<String>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    ctx.insert("CONFIG", config.get_ref());
    ctx.insert("NODE_VERSION", node_version.get_ref());

    let conn = pool.get().expect("couldn't get db connection from pool");

    let templates_and_blocks_with_sanctioned_tx =
        web::block(move || db::debug_templates_and_blocks_with_sanctioned_tx(&conn))
            .await
            .map_err(error::database_error)?;
    ctx.insert(
        "templates_and_blocks_with_sanctioned_tx",
        &templates_and_blocks_with_sanctioned_tx,
    );

    let s = tmpl
        .render("debug/rss/sanctioned.rss", &ctx)
        .map_err(error::template_error)?;
    Ok(HttpResponse::Ok()
        .content_type("application/rss+xml")
        .body(s))
}
