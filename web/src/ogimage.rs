use actix_web::{error as actix_error, web, Error, HttpResponse, Result};
use miningpool_observer_shared::{config, db_pool, tags};
use serde::Serialize;
use tiny_skia::Pixmap;

use std::convert::TryFrom;

use crate::{db, error, util};

fn format_tag(tag: &tags::Tag) -> String {
    let color = match tag.color {
        tags::CYAN => "#00a6d6",
        tags::YELLOW => "#FFA500",
        tags::RED => "#cb2821",
        _ => "#000000",
    }
    .to_string();
    format!("<tspan fill=\"{}\">{}</tspan>", color, tag.name)
}

pub async fn ogimage_missing_transaction(
    txid_str: web::Path<String>,
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    usvg_opts: web::Data<usvg::Options>,
) -> Result<HttpResponse, Error> {
    let txid = util::parse_txid_str(&txid_str)?;
    let mut ctx = tera::Context::new();

    let conn = pool.get().expect("couldn't get db connection from pool");
    let missing_transaction = web::block(move || db::single_missing_transaction(&txid, &conn))
        .await?
        .map_err(actix_web::error::ErrorInternalServerError)?;

    #[derive(Serialize)]
    struct Data {
        block_count: usize,
        txid: String,
        feerate: f32,
        size: i32,
        fee: i64,
        tags: Vec<String>,
    }

    ctx.insert(
        "data",
        &Data {
            block_count: missing_transaction.blocks.len(),
            txid: hex::encode(missing_transaction.transaction.txid),
            feerate: ((missing_transaction.transaction.fee as f64
                / missing_transaction.transaction.vsize as f64) as f32)
                .round(),
            fee: missing_transaction.transaction.fee,
            size: missing_transaction.transaction.vsize,
            tags: missing_transaction
                .transaction
                .tags
                .iter()
                .map(|id| format_tag(&tags::TxTag::try_from(*id).unwrap().value()))
                .collect(),
        },
    );
    ctx.insert("config", config.get_ref());

    let s = tmpl
        .render("svg/subpage_missing_transaction.svg", &ctx)
        .map_err(error::template_error)?;

    match render_and_encode(&s, usvg_opts.get_ref()) {
        Ok(png_data) => Ok(HttpResponse::Ok().content_type("image/png").body(png_data)),
        Err(e) => {
            log::error!(
                "Could not render og::image subpage_missing_transaction: {}",
                e
            );
            Err(actix_error::ErrorInternalServerError(e))
        }
    }
}

pub async fn ogimage_template_and_block(
    hash_str: web::Path<String>,
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    usvg_opts: web::Data<usvg::Options>,
) -> Result<HttpResponse, Error> {
    let hash = util::parse_block_hash_str(&hash_str)?;
    let mut ctx = tera::Context::new();
    ctx.insert("config", config.get_ref());

    let conn = pool.get().expect("couldn't get db connection from pool");
    let block = web::block(move || db::block(&hash, &conn))
        .await?
        .map_err(actix_web::error::ErrorInternalServerError)?;

    #[derive(Serialize)]
    struct Data {
        hash: String,
        height: i32,
        pool: String,
        missing: i32,
        extra: i32,
        shared: i32,
    }

    ctx.insert(
        "data",
        &Data {
            hash: hex::encode(block.hash),
            height: block.height,
            pool: block.pool_name,
            missing: block.missing_tx,
            extra: block.extra_tx,
            shared: block.shared_tx,
        },
    );

    let s = tmpl
        .render("svg/subpage_template_and_block.svg", &ctx)
        .map_err(error::template_error)?;

    match render_and_encode(&s, usvg_opts.get_ref()) {
        Ok(png_data) => Ok(HttpResponse::Ok().content_type("image/png").body(png_data)),
        Err(e) => {
            log::error!(
                "Could not render og::image subpage_template_and_block: {}",
                e
            );
            Err(actix_error::ErrorInternalServerError(
                "Render or Encoding Error",
            ))
        }
    }
}

pub async fn ogimage_block_with_conflicting_transactions(
    hash_str: web::Path<String>,
    tmpl: web::Data<tera::Tera>,
    pool: web::Data<db_pool::PgPool>,
    config: web::Data<config::WebSiteConfig>,
    usvg_opts: web::Data<usvg::Options>,
) -> Result<HttpResponse, Error> {
    let hash = util::parse_block_hash_str(&hash_str)?;
    let mut ctx = tera::Context::new();
    ctx.insert("config", config.get_ref());

    let conn = pool.get().expect("couldn't get db connection from pool");
    let block_with_conflicting_transactions =
        web::block(move || db::single_block_with_conflicting_transactions(&conn, &hash))
            .await?
            .map_err(actix_web::error::ErrorInternalServerError)?;

    #[derive(Serialize)]
    struct Data {
        hash: String,
        pool: String,
        height: i32,
        conflicts: usize,
    }

    ctx.insert(
        "data",
        &Data {
            hash: hex::encode(block_with_conflicting_transactions.block.hash),
            pool: block_with_conflicting_transactions.block.pool_name,
            height: block_with_conflicting_transactions.block.height,
            conflicts: block_with_conflicting_transactions
                .conflicting_transaction_sets
                .len(),
        },
    );

    let s = tmpl
        .render("svg/subpage_block_with_conflicting_transactions.svg", &ctx)
        .map_err(error::template_error)?;

    match render_and_encode(&s, usvg_opts.get_ref()) {
        Ok(png_data) => Ok(HttpResponse::Ok().content_type("image/png").body(png_data)),
        Err(e) => {
            log::error!(
                "Could not render og::image subpage_block_with_conflicting_transactions: {}",
                e
            );
            Err(actix_error::ErrorInternalServerError(
                "Render or Encoding Error",
            ))
        }
    }
}

pub async fn ogimage_mainpage_templates_and_blocks(
    config: web::Data<config::WebSiteConfig>,
    usvg_opts: web::Data<usvg::Options>,
    tmpl: web::Data<tera::Tera>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    ctx.insert("config", config.get_ref());
    let s = tmpl
        .render("svg/mainpage_templates_and_blocks.svg", &ctx)
        .map_err(error::template_error)?;

    match render_and_encode(&s, usvg_opts.get_ref()) {
        Ok(png_data) => Ok(HttpResponse::Ok().content_type("image/png").body(png_data)),
        Err(e) => {
            log::error!(
                "Could not render og::image mainpage_templates_and_blocks: {}",
                e
            );
            Err(actix_error::ErrorInternalServerError(
                "Render or Encoding Error",
            ))
        }
    }
}

pub async fn ogimage_mainpage_faq(
    config: web::Data<config::WebSiteConfig>,
    usvg_opts: web::Data<usvg::Options>,
    tmpl: web::Data<tera::Tera>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    ctx.insert("config", config.get_ref());
    let s = tmpl
        .render("svg/mainpage_faq.svg", &ctx)
        .map_err(error::template_error)?;

    match render_and_encode(&s, usvg_opts.get_ref()) {
        Ok(png_data) => Ok(HttpResponse::Ok().content_type("image/png").body(png_data)),
        Err(e) => {
            log::error!("Could not render og::mainpage_faq: {}", e);
            Err(actix_error::ErrorInternalServerError(
                "Render or Encoding Error",
            ))
        }
    }
}

pub async fn ogimage_mainpage_missing_transactions(
    config: web::Data<config::WebSiteConfig>,
    usvg_opts: web::Data<usvg::Options>,
    tmpl: web::Data<tera::Tera>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    ctx.insert("config", config.get_ref());
    let s = tmpl
        .render("svg/mainpage_missing_transactions.svg", &ctx)
        .map_err(error::template_error)?;

    match render_and_encode(&s, usvg_opts.get_ref()) {
        Ok(png_data) => Ok(HttpResponse::Ok().content_type("image/png").body(png_data)),
        Err(e) => {
            log::error!(
                "Could not render og::image mainpage_missing_transactions: {}",
                e
            );
            Err(actix_error::ErrorInternalServerError(
                "Render or Encoding Error",
            ))
        }
    }
}

pub async fn ogimage_mainpage_conflicting_transactions(
    config: web::Data<config::WebSiteConfig>,
    usvg_opts: web::Data<usvg::Options>,
    tmpl: web::Data<tera::Tera>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    ctx.insert("config", config.get_ref());
    let s = tmpl
        .render("svg/mainpage_conflicting_transactions.svg", &ctx)
        .map_err(error::template_error)?;

    match render_and_encode(&s, usvg_opts.get_ref()) {
        Ok(png_data) => Ok(HttpResponse::Ok().content_type("image/png").body(png_data)),
        Err(e) => {
            log::error!(
                "Could not render og::image mainpage_conflicting_transactions: {}",
                e
            );
            Err(actix_error::ErrorInternalServerError(
                "Render or Encoding Error",
            ))
        }
    }
}

pub async fn ogimage_mainpage_index(
    config: web::Data<config::WebSiteConfig>,
    usvg_opts: web::Data<usvg::Options>,
    tmpl: web::Data<tera::Tera>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    ctx.insert("config", config.get_ref());
    let s = tmpl
        .render("svg/mainpage_index.svg", &ctx)
        .map_err(error::template_error)?;

    match render_and_encode(&s, usvg_opts.get_ref()) {
        Ok(png_data) => Ok(HttpResponse::Ok().content_type("image/png").body(png_data)),
        Err(e) => {
            log::error!("Could not render og::image mainpage_index: {}", e);
            Err(actix_error::ErrorInternalServerError(
                "Render or Encoding Error",
            ))
        }
    }
}

fn render_and_encode(svg: &str, opts: &usvg::Options) -> Result<Vec<u8>, ImageError> {
    let rtree = usvg::Tree::from_str(svg, opts)?;
    let size = rtree.svg_node().size.to_screen_size();
    let mut pixmap = match Pixmap::new(size.width(), size.height()) {
        Some(pixmap) => pixmap,
        None => return Err(ImageError::PixmapCreationError),
    };
    if resvg::render(&rtree, usvg::FitTo::Original, pixmap.as_mut()).is_none() {
        return Err(ImageError::RenderError);
    }
    match Pixmap::encode_png(&pixmap) {
        Ok(png_data) => Ok(png_data),
        Err(e) => Err(ImageError::PngEncodingError(e.to_string())),
    }
}

#[derive(Debug)]
pub enum ImageError {
    UsvgError(usvg::Error),
    PngEncodingError(String),
    RenderError,
    PixmapCreationError,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        match *self {
            ImageError::UsvgError(ref e) => e.fmt(f),
            ImageError::PngEncodingError(ref e) => write!(f, "could not encoded as PNG: {}", e),
            ImageError::RenderError => write!(f, "could not render the SVG"),
            ImageError::PixmapCreationError => write!(f, "could not create a new PixMap"),
        }
    }
}

impl std::error::Error for ImageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            ImageError::UsvgError(ref e) => Some(e),
            ImageError::RenderError => None,
            ImageError::PngEncodingError(_) => None,
            ImageError::PixmapCreationError => None,
        }
    }
}

impl From<usvg::Error> for ImageError {
    fn from(e: usvg::Error) -> Self {
        ImageError::UsvgError(e)
    }
}
