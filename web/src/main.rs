#[macro_use]
extern crate diesel;

mod db;
mod error;
mod handler;
mod model;
mod ogimage;
mod util;

use actix_web::http::StatusCode;
use actix_web::middleware::errhandlers::ErrorHandlers;
use actix_web::{middleware, web, App, HttpServer};
use simple_logger::SimpleLogger;
use tera::Tera;

use miningpool_observer_shared::{config, db_pool};

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    std::env::set_var("RUST_LOG", "actix_web=info");

    let config = config::load_web_config().expect("could not load the configuration");
    let cloned_config = config.clone();

    match SimpleLogger::new().with_utc_timestamps().with_level(config.log_level).init() {
        Ok(_) => (),
        Err(e) => panic!("Could not setup logger: {}", e),
    }

    let pool = match db_pool::new(&config.database_url) {
        Ok(pool) => pool,
        Err(e) => panic!("Could not create a Postgres connection pool: {}", e),
    };
    log::info!(target: "startup", "Successfully created a database connection pool with a max size of {} connections.", pool.max_size());

    HttpServer::new(move || {
        let mut tera = match Tera::new(&(cloned_config.www_dir_path.clone() + "/templates/**/*")) {
            Ok(tera) => tera,
            Err(error) => {
                log::error!("Tera template parsing failed: {}", error);
                panic!("Tera template parsing failed.");
            }
        };
        tera.register_function("block_tag_id_to_tag", util::block_tag_id_to_tag());
        tera.register_function("tx_tag_id_to_tag", util::tx_tag_id_to_tag());
        tera.register_function("seconds_to_duration", util::seconds_to_duration());

        let conn = pool.clone().get().unwrap();
        let node_version = db::get_node_info(&conn).unwrap();

        let usvg_options: usvg::Options = {
            let mut opt = usvg::Options::default();
            opt.fontdb
                .load_fonts_dir(cloned_config.www_dir_path.clone() + "/static/fonts");
            opt.fontdb.set_sans_serif_family("Roboto");
            opt.fontdb.set_monospace_family("Roboto Mono");
            opt
        };

        App::new()
            .data(tera)
            .data(pool.clone())
            .data(cloned_config.site.clone())
            .data(cloned_config.debug_pages)
            .data(usvg_options)
            .data(node_version)
            .wrap(middleware::Logger::default())
            //
            // ERROR HANDLING
            //
            .wrap(
                ErrorHandlers::new()
                    .handler(StatusCode::NOT_FOUND, error::not_found)
                    .handler(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        error::internal_server_error,
                    )
                    .handler(StatusCode::UNAUTHORIZED, error::unauthorized),
            )
            //
            // INDEX
            //
            .route("/", web::get().to(handler::index))
            .route(
                "/og_image/index.png",
                web::get().to(ogimage::ogimage_mainpage_index),
            )
            //
            // TEMPLATE & BLOCK PAGES
            //
            .route(
                "/template-and-block",
                web::get().to(handler::templates_and_blocks),
            )
            .route(
                "/template-and-block/sanctioned-feed.xml",
                web::get().to(handler::missing_sanctioned_transactions_rss),
            )
            .route(
                "/og_image/template-and-block.png",
                web::get().to(ogimage::ogimage_mainpage_templates_and_blocks),
            )
            .route(
                "/template-and-block/{hash}",
                web::get().to(handler::single_template_and_block),
            )
            .route(
                "/og_image/template-and-block/{hash}.png",
                web::get().to(ogimage::ogimage_template_and_block),
            )
            //
            // MISSING TRANSACTION PAGES
            //
            .route("/missing", web::get().to(handler::missing_transactions))
            .route(
                "/og_image/missing.png",
                web::get().to(ogimage::ogimage_mainpage_missing_transactions),
            )
            .route(
                "/missing/{txid}",
                web::get().to(handler::single_missing_transaction),
            )
            .route(
                "/og_image/missing/{txid}.png",
                web::get().to(ogimage::ogimage_missing_transaction),
            )
            //
            // CONFLICTING TRANSACTION PAGES
            //
            .route(
                "/conflicting",
                web::get().to(handler::conflicting_transactions),
            )
            .route(
                "/og_image/conflicting.png",
                web::get().to(ogimage::ogimage_mainpage_conflicting_transactions),
            )
            .route(
                "/conflicting/{hash}",
                web::get().to(handler::single_block_with_conflicting_transactions),
            )
            .route(
                "/og_image/conflicting/{hash}.png",
                web::get().to(ogimage::ogimage_block_with_conflicting_transactions),
            )
            //
            // DEBUG PAGES
            //
            .route("/debug", web::get().to(handler::debug))
            .route("/debug/fees", web::get().to(handler::debug_fees_by_pool))
            .route(
                "/debug/unknown",
                web::get().to(handler::debug_unknown_pool_blocks),
            )
            .route(
                "/debug/sanctioned-utxo-scans",
                web::get().to(handler::debug_utxoset_scans),
            )
            .route(
                "/debug/template-selection",
                web::get().to(handler::debug_template_selection_infos),
            )
            .route(
                "/debug/sanctioned",
                web::get().to(handler::debug_sanctioned_by_pool),
            )
            .route(
                "/debug/sanctioned/feed.xml",
                web::get().to(handler::debug_sanctioned_transactions_rss),
            )
            //
            // OTHER PAGES
            //
            .route("/faq", web::get().to(handler::faq))
            .route(
                "/og_image/faq.png",
                web::get().to(ogimage::ogimage_mainpage_faq),
            )
            .route("/robots.txt", web::get().to(handler::robots_txt))
            //
            // STATIC FILES
            //
            .service(actix_files::Files::new(
                "/static",
                cloned_config.www_dir_path.clone() + "/static",
            ))
    })
    .bind(config.address)?
    .run()
    .await
}
