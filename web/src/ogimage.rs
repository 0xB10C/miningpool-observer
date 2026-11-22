use actix_web::{error as actix_error, web, Error, HttpResponse, Result};
use miningpool_observer_shared::{config, db_pool, tags};
use resvg::tiny_skia::{self, Pixmap};
use resvg::usvg;
use serde::Serialize;

use std::convert::TryFrom;

use crate::model::MissingTransaction;
use crate::{db, error, util};

/// The data the `svg/subpage_missing_transaction.svg` template renders.
#[derive(Serialize)]
struct MissingTransactionData {
    block_count: usize,
    txid: String,
    feerate: f32,
    size: i32,
    fee: i64,
    tags: Vec<String>,
}

fn missing_transaction_og_data(mt: &MissingTransaction) -> MissingTransactionData {
    MissingTransactionData {
        block_count: mt.blocks.len(),
        txid: hex::encode(&mt.transaction.txid),
        feerate: ((mt.transaction.fee as f64 / mt.transaction.vsize as f64) as f32).round(),
        fee: mt.transaction.fee,
        size: mt.transaction.vsize,
        // Tag ids that we don't know about anymore (a tag can be deprecated
        // and its id reused) are skipped rather than rendered.
        tags: mt
            .transaction
            .tags
            .iter()
            .filter_map(|id| tags::TxTag::try_from(*id).ok())
            .map(|tag| format_tag(&tag.value()))
            .collect(),
    }
}

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
    usvg_opts: web::Data<usvg::Options<'static>>,
) -> Result<HttpResponse, Error> {
    let txid = util::parse_txid_str(&txid_str)?;
    let mut ctx = tera::Context::new();

    let mut conn = pool.get().expect("couldn't get db connection from pool");
    let missing_transaction = web::block(move || db::single_missing_transaction(&txid, &mut conn))
        .await?
        .map_err(actix_web::error::ErrorInternalServerError)?;

    ctx.insert("data", &missing_transaction_og_data(&missing_transaction));
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
    usvg_opts: web::Data<usvg::Options<'static>>,
) -> Result<HttpResponse, Error> {
    let hash = util::parse_block_hash_str(&hash_str)?;
    let mut ctx = tera::Context::new();
    ctx.insert("config", config.get_ref());

    let mut conn = pool.get().expect("couldn't get db connection from pool");
    let block = web::block(move || db::block(&hash, &mut conn))
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
    usvg_opts: web::Data<usvg::Options<'static>>,
) -> Result<HttpResponse, Error> {
    let hash = util::parse_block_hash_str(&hash_str)?;
    let mut ctx = tera::Context::new();
    ctx.insert("config", config.get_ref());

    let mut conn = pool.get().expect("couldn't get db connection from pool");
    let block_with_conflicting_transactions =
        web::block(move || db::single_block_with_conflicting_transactions(&mut conn, &hash))
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
    usvg_opts: web::Data<usvg::Options<'static>>,
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
    usvg_opts: web::Data<usvg::Options<'static>>,
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
    usvg_opts: web::Data<usvg::Options<'static>>,
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
    usvg_opts: web::Data<usvg::Options<'static>>,
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

pub async fn ogimage_mainpage_sanctioned_transactions(
    config: web::Data<config::WebSiteConfig>,
    usvg_opts: web::Data<usvg::Options<'static>>,
    tmpl: web::Data<tera::Tera>,
) -> Result<HttpResponse, Error> {
    let mut ctx = tera::Context::new();
    ctx.insert("config", config.get_ref());
    let s = tmpl
        .render("svg/mainpage_sanctioned_transactions.svg", &ctx)
        .map_err(error::template_error)?;

    match render_and_encode(&s, usvg_opts.get_ref()) {
        Ok(png_data) => Ok(HttpResponse::Ok().content_type("image/png").body(png_data)),
        Err(e) => {
            log::error!(
                "Could not render og::image mainpage_sanctioned_transactions: {}",
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
    usvg_opts: web::Data<usvg::Options<'static>>,
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

pub(crate) fn render_and_encode(
    svg: &str,
    opts: &usvg::Options<'static>,
) -> Result<Vec<u8>, ImageError> {
    let rtree = usvg::Tree::from_str(svg, opts)?;
    let size = rtree.size().to_int_size();
    let mut pixmap = match Pixmap::new(size.width(), size.height()) {
        Some(pixmap) => pixmap,
        None => return Err(ImageError::PixmapCreationError),
    };
    resvg::render(
        &rtree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    match Pixmap::encode_png(&pixmap) {
        Ok(png_data) => Ok(png_data),
        Err(e) => Err(ImageError::PngEncodingError(e.to_string())),
    }
}

#[derive(Debug)]
pub enum ImageError {
    UsvgError(usvg::Error),
    PngEncodingError(String),
    PixmapCreationError,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        match *self {
            ImageError::UsvgError(ref e) => e.fmt(f),
            ImageError::PngEncodingError(ref e) => write!(f, "could not encoded as PNG: {}", e),
            ImageError::PixmapCreationError => write!(f, "could not create a new PixMap"),
        }
    }
}

impl std::error::Error for ImageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            ImageError::UsvgError(ref e) => Some(e),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{missing_transaction_block_info_fixture, transaction_fixture};
    use std::convert::TryInto;

    const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

    fn tag(color: &'static str) -> tags::Tag {
        tags::Tag {
            name: "Test Tag".to_string(),
            description: vec!["A tag used in the tests.".to_string()],
            color,
            text_color: tags::WHITE,
        }
    }

    #[test]
    fn test_format_tag() {
        assert_eq!(
            format_tag(&tag(tags::CYAN)),
            "<tspan fill=\"#00a6d6\">Test Tag</tspan>"
        );
        assert_eq!(
            format_tag(&tag(tags::YELLOW)),
            "<tspan fill=\"#FFA500\">Test Tag</tspan>"
        );
        assert_eq!(
            format_tag(&tag(tags::RED)),
            "<tspan fill=\"#cb2821\">Test Tag</tspan>"
        );
    }

    #[test]
    fn test_format_tag_falls_back_to_black() {
        // Colors without a mapping are rendered black.
        assert_eq!(
            format_tag(&tag(tags::GRAY)),
            "<tspan fill=\"#000000\">Test Tag</tspan>"
        );
        assert_eq!(
            format_tag(&tag(tags::GREEN)),
            "<tspan fill=\"#000000\">Test Tag</tspan>"
        );
    }

    #[test]
    fn test_render_and_encode() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <rect width="10" height="10" fill="#00a6d6"/>
        </svg>"##;
        let png = render_and_encode(svg, &usvg::Options::default()).unwrap();

        assert_eq!(&png[..8], &PNG_SIGNATURE);
        // The IHDR chunk starts at byte 8 and has the width and the height as
        // big-endian u32 at byte 16 and 20.
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 10);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 10);
    }

    #[test]
    fn test_render_and_encode_malformed_svg() {
        match render_and_encode("this is not an SVG", &usvg::Options::default()) {
            Err(ImageError::UsvgError(_)) => (),
            other => panic!("expected a UsvgError, got {:?}", other),
        }
    }

    #[test]
    fn test_render_and_encode_zero_sized_svg() {
        // A zero-sized SVG would also make the Pixmap creation fail, but usvg
        // already rejects it while parsing. Either way, this must not panic.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"></svg>"#;
        match render_and_encode(svg, &usvg::Options::default()) {
            Err(ImageError::UsvgError(usvg::Error::InvalidSize)) => (),
            other => panic!("expected an InvalidSize error, got {:?}", other),
        }
    }

    #[test]
    fn test_image_error_display() {
        let usvg_error = ImageError::from(usvg::Error::InvalidSize);
        assert!(!usvg_error.to_string().is_empty());
        assert!(!ImageError::PngEncodingError("oops".to_string())
            .to_string()
            .is_empty());
        assert!(ImageError::PngEncodingError("oops".to_string())
            .to_string()
            .contains("oops"));
        assert!(!ImageError::PixmapCreationError.to_string().is_empty());
    }

    #[test]
    fn test_image_error_source() {
        use std::error::Error;

        // only the usvg error wraps another error
        assert!(ImageError::from(usvg::Error::InvalidSize)
            .source()
            .is_some());
        assert!(ImageError::PngEncodingError("oops".to_string())
            .source()
            .is_none());
        assert!(ImageError::PixmapCreationError.source().is_none());
    }

    fn missing_transaction(tags: Vec<i32>) -> MissingTransaction {
        MissingTransaction {
            transaction: transaction_fixture(tags),
            blocks: vec![
                missing_transaction_block_info_fixture(700_000),
                missing_transaction_block_info_fixture(700_001),
            ],
        }
    }

    #[test]
    fn test_missing_transaction_og_data() {
        let data =
            missing_transaction_og_data(&missing_transaction(vec![tags::TxTag::Young as i32]));

        assert_eq!(data.block_count, 2);
        assert_eq!(data.txid, hex::encode(vec![0xaa; 32]));
        assert_eq!(data.fee, 1000);
        assert_eq!(data.size, 300);
        // 1000 sat / 300 vByte = 3.33 sat/vByte, rounded to 3
        assert_eq!(data.feerate, 3.0);
        assert_eq!(data.tags, vec![format_tag(&tags::TxTag::Young.value())]);
    }

    #[test]
    fn test_missing_transaction_og_data_skips_unknown_tags() {
        // 3100 is a deprecated tag id. Transactions tagged with it are still
        // in the database, but it can't be mapped to a tag anymore.
        let data = missing_transaction_og_data(&missing_transaction(vec![
            3100,
            tags::TxTag::Coinbase as i32,
        ]));

        assert_eq!(data.tags, vec![format_tag(&tags::TxTag::Coinbase.value())]);
    }

    #[test]
    fn test_missing_transaction_og_data_without_tags() {
        let data = missing_transaction_og_data(&missing_transaction(vec![]));
        assert!(data.tags.is_empty());
    }

    /// The OpenGraph images of the main pages are rendered without touching
    /// the database. Requesting them has to return a PNG.
    #[actix_web::test]
    async fn test_mainpage_ogimage_handlers() {
        use crate::test_util::{site_config, tera, usvg_options};
        use actix_web::http::{header, StatusCode};
        use actix_web::web::Data;
        use actix_web::{test, App};

        let app = test::init_service(
            App::new()
                .app_data(Data::new(tera()))
                .app_data(Data::new(site_config()))
                .app_data(Data::new(usvg_options()))
                .route("/index.png", web::get().to(ogimage_mainpage_index))
                .route("/faq.png", web::get().to(ogimage_mainpage_faq))
                .route(
                    "/template-and-block.png",
                    web::get().to(ogimage_mainpage_templates_and_blocks),
                )
                .route(
                    "/missing.png",
                    web::get().to(ogimage_mainpage_missing_transactions),
                )
                .route(
                    "/conflicting.png",
                    web::get().to(ogimage_mainpage_conflicting_transactions),
                )
                .route(
                    "/missing-sanctioned.png",
                    web::get().to(ogimage_mainpage_sanctioned_transactions),
                ),
        )
        .await;

        for uri in [
            "/index.png",
            "/faq.png",
            "/template-and-block.png",
            "/missing.png",
            "/conflicting.png",
            "/missing-sanctioned.png",
        ]
        .iter()
        {
            let req = test::TestRequest::get().uri(uri).to_request();
            let res = test::call_service(&app, req).await;

            assert_eq!(res.status(), StatusCode::OK, "{}", uri);
            assert_eq!(
                res.headers().get(header::CONTENT_TYPE).unwrap(),
                "image/png",
                "{}",
                uri
            );
            let body = test::read_body(res).await;
            assert_eq!(&body[..8], &PNG_SIGNATURE, "{}", uri);
        }
    }
}
