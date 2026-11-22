//! Integration tests for the request handlers that look up a block or a
//! transaction by hash.
//!
//! The handlers get their database connections from a pool, so they can't run
//! in a test transaction like the queries in db_test.rs. These tests only make
//! requests for hashes that aren't in the database, which read but never
//! write, so they don't disturb the other database tests.
//!
//! They need a PostgreSQL database and are skipped when the environment
//! variable TEST_DATABASE_URL isn't set. `contrib/test-db.sh` starts a
//! throwaway database.

use actix_web::http::StatusCode;
use actix_web::middleware::ErrorHandlers;
use actix_web::web::Data;
use actix_web::{test, web, App};

use crate::test_util::{node_version, site_config, tera, test_pool, usvg_options};
use crate::{error, handler, ogimage};

/// Returns a connection pool for the test database or leaves the test early
/// when there is no test database.
macro_rules! pool_or_skip {
    () => {
        match test_pool() {
            Some(pool) => pool,
            None => return,
        }
    };
}

/// A hash and a txid that no block and no transaction in the database has.
const UNKNOWN_HASH: &str = "00000000000000000000000000000000000000000000000000000000deadbeef";

/// The app set up like in main.rs: the routes are registered first and the
/// scope the error handlers are wrapped around comes last. It's a macro
/// because the type of a scope with a wrapped middleware is hard to write
/// down.
macro_rules! app {
    ($pool:expr) => {
        App::new()
            .app_data(Data::new(tera()))
            .app_data(Data::new($pool))
            .app_data(Data::new(site_config()))
            .app_data(Data::new(node_version()))
            .app_data(Data::new(usvg_options()))
            .route(
                "/template-and-block/{hash}",
                web::get().to(handler::single_template_and_block),
            )
            .route(
                "/missing/{txid}",
                web::get().to(handler::single_missing_transaction),
            )
            .route(
                "/conflicting/{hash}",
                web::get().to(handler::single_block_with_conflicting_transactions),
            )
            .route(
                "/og_image/template-and-block/{hash}.png",
                web::get().to(ogimage::ogimage_template_and_block),
            )
            .route(
                "/og_image/missing/{txid}.png",
                web::get().to(ogimage::ogimage_missing_transaction),
            )
            .service(
                web::scope("").wrap(
                    ErrorHandlers::new()
                        .handler(StatusCode::NOT_FOUND, error::not_found)
                        .handler(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            error::internal_server_error,
                        )
                        .handler(StatusCode::UNAUTHORIZED, error::unauthorized),
                ),
            )
    };
}

#[actix_web::test]
async fn test_a_hash_that_is_not_in_the_database_answers_with_a_404() {
    let pool = pool_or_skip!();
    let app = test::init_service(app!(pool)).await;

    for uri in [
        format!("/template-and-block/{}", UNKNOWN_HASH),
        format!("/missing/{}", UNKNOWN_HASH),
        format!("/conflicting/{}", UNKNOWN_HASH),
        format!("/og_image/template-and-block/{}.png", UNKNOWN_HASH),
        format!("/og_image/missing/{}.png", UNKNOWN_HASH),
    ]
    .iter()
    {
        let req = test::TestRequest::get().uri(uri).to_request();
        let res = test::call_service(&app, req).await;

        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{}", uri);

        // The ErrorHandlers middleware only sees the responses of the scope
        // it's wrapped around. main.rs registers the routes outside of that
        // scope, so an error a handler returns keeps its plain text body
        // instead of being rendered as the error page.
        let body = String::from_utf8(test::read_body(res).await.to_vec()).unwrap();
        assert_eq!(body, "Not found", "{}", uri);
    }
}

#[actix_web::test]
async fn test_a_path_without_a_route_renders_the_error_page() {
    let pool = pool_or_skip!();
    let app = test::init_service(app!(pool)).await;

    // A request that doesn't match any route falls through to the scope the
    // error handlers are wrapped around, so it gets the error page.
    let req = test::TestRequest::get()
        .uri("/template-and-block/not-a-page/at-all")
        .to_request();
    let res = test::call_service(&app, req).await;

    assert_eq!(res.status(), StatusCode::NOT_FOUND);
    let body = String::from_utf8(test::read_body(res).await.to_vec()).unwrap();
    assert!(body.contains("Page not found"), "{}", body);
}
