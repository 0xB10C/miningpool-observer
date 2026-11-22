use miningpool_observer_shared::config;
use miningpool_observer_shared::diesel;
use tera::Tera;

use actix_web::{
    body::BoxBody, dev::ServiceResponse, http::header::ContentType,
    middleware::ErrorHandlerResponse, web, HttpResponse, Result,
};

pub fn template_error(e: tera::Error) -> actix_web::Error {
    log::error!("Template Error: {}", e);
    actix_web::error::ErrorInternalServerError("Template Error")
}

/// Turns a database error into a response error. A query for a block hash or
/// a txid that isn't in the database answers with a 404 instead of a 500: a
/// visitor asking for something we don't have isn't a server error.
pub fn db_error(e: diesel::result::Error) -> actix_web::Error {
    match e {
        diesel::result::Error::NotFound => actix_web::error::ErrorNotFound("Not found"),
        e => {
            log::error!("Database Error: {}", e);
            actix_web::error::ErrorInternalServerError("Database Error")
        }
    }
}

pub fn unauthorized<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<BoxBody>> {
    let response = get_error_response(&res, "Unauthorized");
    Ok(ErrorHandlerResponse::Response(ServiceResponse::new(
        res.into_parts().0,
        response.map_into_left_body(),
    )))
}

pub fn not_found<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<BoxBody>> {
    let response = get_error_response(&res, "Page not found");
    Ok(ErrorHandlerResponse::Response(ServiceResponse::new(
        res.into_parts().0,
        response.map_into_left_body(),
    )))
}

pub fn internal_server_error<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<BoxBody>> {
    let response = get_error_response(&res, "Internal Server Error");
    Ok(ErrorHandlerResponse::Response(ServiceResponse::new(
        res.into_parts().0,
        response.map_into_left_body(),
    )))
}

fn get_error_response<B>(res: &ServiceResponse<B>, error: &str) -> HttpResponse<BoxBody> {
    let request = res.request();

    // Provide a fallback to a simple plain text response in case an error occurs during the
    // rendering of the error page.
    let fallback = |e: &str| {
        HttpResponse::build(res.status())
            .content_type(ContentType::plaintext())
            .body(e.to_string())
    };

    let tmpl = request.app_data::<web::Data<Tera>>().map(|t| t.get_ref());
    let config = request
        .app_data::<web::Data<config::WebSiteConfig>>()
        .map(|t| t.get_ref());
    // The error page extends the base template, which shows the node version
    // in the footer. Without it, rendering the error page fails.
    let node_version = request
        .app_data::<web::Data<String>>()
        .map(|v| v.get_ref().clone())
        .unwrap_or_default();
    match tmpl {
        Some(tmpl) => {
            let mut context = tera::Context::new();
            context.insert("error", error);
            context.insert("status_code", res.status().as_str());
            context.insert("CONFIG", &config);
            context.insert("NODE_VERSION", &node_version);

            let body = tmpl.render("error.html", &context);
            match body {
                Ok(body) => HttpResponse::build(res.status())
                    .content_type(ContentType::html())
                    .body(body),
                Err(e) => {
                    log::error!("Could not render the error page: {}", e);
                    fallback(error)
                }
            }
        }
        None => fallback(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{node_version, site_config, tera};

    use actix_web::http::{header, StatusCode};
    use actix_web::middleware::ErrorHandlers;
    use actix_web::web::Data;
    use actix_web::{test, App, HttpResponse};

    /// The error handlers set up like in main.rs.
    fn error_handlers() -> ErrorHandlers<BoxBody> {
        ErrorHandlers::new()
            .handler(StatusCode::NOT_FOUND, not_found)
            .handler(StatusCode::INTERNAL_SERVER_ERROR, internal_server_error)
            .handler(StatusCode::UNAUTHORIZED, unauthorized)
    }

    /// The scope the error handlers are wrapped around, with a route for the
    /// status codes that have a handler. It's a macro because the type of a
    /// scope with a wrapped middleware is hard to write down.
    macro_rules! scope {
        () => {
            web::scope("")
                .wrap(error_handlers())
                .route(
                    "/unauthorized",
                    web::get().to(|| async { HttpResponse::Unauthorized().finish() }),
                )
                .route(
                    "/server-error",
                    web::get().to(|| async { HttpResponse::InternalServerError().finish() }),
                )
        };
    }

    async fn body_of<B>(res: ServiceResponse<B>) -> String
    where
        B: actix_web::body::MessageBody,
    {
        String::from_utf8(test::read_body(res).await.to_vec()).unwrap()
    }

    #[actix_web::test]
    async fn test_not_found_renders_the_error_page() {
        let app = test::init_service(
            App::new()
                .app_data(Data::new(tera()))
                .app_data(Data::new(site_config()))
                .app_data(Data::new(node_version()))
                .service(scope!()),
        )
        .await;

        let req = test::TestRequest::get().uri("/does-not-exist").to_request();
        let res = test::call_service(&app, req).await;

        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );

        let body = body_of(res).await;
        assert!(body.contains("Page not found"), "{}", body);
        assert!(body.contains("404"), "{}", body);
        assert!(body.contains("Test Observer"), "{}", body);
    }

    #[actix_web::test]
    async fn test_unauthorized_and_server_error_render_the_error_page() {
        let app = test::init_service(
            App::new()
                .app_data(Data::new(tera()))
                .app_data(Data::new(site_config()))
                .app_data(Data::new(node_version()))
                .service(scope!()),
        )
        .await;

        let req = test::TestRequest::get().uri("/unauthorized").to_request();
        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        let body = body_of(res).await;
        assert!(body.contains("Unauthorized"), "{}", body);
        assert!(body.contains("401"), "{}", body);

        let req = test::TestRequest::get().uri("/server-error").to_request();
        let res = test::call_service(&app, req).await;
        assert_eq!(res.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_of(res).await;
        assert!(body.contains("Internal Server Error"), "{}", body);
        assert!(body.contains("500"), "{}", body);
    }

    #[actix_web::test]
    async fn test_not_found_without_templates() {
        // Without a Tera instance the error page can't be rendered. The
        // handler falls back to a plain text response.
        let app = test::init_service(App::new().service(scope!())).await;

        let req = test::TestRequest::get().uri("/does-not-exist").to_request();
        let res = test::call_service(&app, req).await;

        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            res.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/plain; charset=utf-8"
        );
        assert_eq!(body_of(res).await, "Page not found");
    }

    #[actix_web::test]
    async fn test_not_found_without_a_site_config() {
        // The site config is needed for the links and the title on the error
        // page. Without it, the plain text fallback is used.
        let app = test::init_service(
            App::new()
                .app_data(Data::new(tera()))
                .app_data(Data::new(node_version()))
                .service(scope!()),
        )
        .await;

        let req = test::TestRequest::get().uri("/does-not-exist").to_request();
        let res = test::call_service(&app, req).await;

        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_of(res).await, "Page not found");
    }
}

/// The tests of [`db_error`] live in their own module: the module above
/// imports actix_web::test, which shadows the #[test] attribute.
#[cfg(test)]
mod db_error_tests {
    use super::*;
    use actix_web::http::StatusCode;

    #[test]
    fn test_db_error_answers_a_missing_row_with_a_404() {
        // A visitor asking for a block or a transaction we don't have isn't a
        // server error.
        let not_found = db_error(diesel::result::Error::NotFound);
        assert_eq!(
            not_found.as_response_error().status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(not_found.to_string(), "Not found");
    }

    #[test]
    fn test_db_error_answers_other_errors_with_a_500() {
        let broken = db_error(diesel::result::Error::BrokenTransactionManager);
        assert_eq!(
            broken.as_response_error().status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(broken.to_string(), "Database Error");

        let rollback = db_error(diesel::result::Error::RollbackTransaction);
        assert_eq!(
            rollback.as_response_error().status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
        assert_eq!(rollback.to_string(), "Database Error");
    }
}
