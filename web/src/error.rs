use actix_http::{body::Body, Response};
use actix_web::dev::ServiceResponse;
use actix_web::middleware::errhandlers::ErrorHandlerResponse;
use actix_web::{error, web, Result};
use diesel::result::Error as DieselError;
use miningpool_observer_shared::config;
use tera::Tera;

pub fn template_error(e: tera::Error) -> actix_web::Error {
    log::error!("Template Error: {}", e);
    actix_web::error::ErrorInternalServerError("Template Error")
}

pub fn database_error(e: error::BlockingError<DieselError>) -> actix_web::Error {
    match e {
        error::BlockingError::Error(e) => {
            log::error!("Database Error: {}", e);
            match e {
                DieselError::NotFound => {
                    actix_web::error::ErrorNotFound("Database error".to_string())
                }
                _ => actix_web::error::ErrorInternalServerError("Database Error"),
            }
        }
        error::BlockingError::Canceled => {
            log::error!("Database Operation Canceled {}", e);
            actix_web::error::ErrorInternalServerError("Database Error")
        }
    }
}

pub fn unauthorized<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
    let response = get_error_response(&res, "Unauthorized");
    Ok(ErrorHandlerResponse::Response(
        res.into_response(response.into_body()),
    ))
}

pub fn not_found<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
    let response = get_error_response(&res, "Not found");
    Ok(ErrorHandlerResponse::Response(
        res.into_response(response.into_body()),
    ))
}

pub fn internal_server_error<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
    let response = get_error_response(&res, "Internal Server Error");
    Ok(ErrorHandlerResponse::Response(
        res.into_response(response.into_body()),
    ))
}

fn get_error_response<B>(res: &ServiceResponse<B>, error: &str) -> Response<Body> {
    let request = res.request();

    // Provide a fallback to a simple plain text response in case an error occurs during the
    // rendering of the error page.
    let fallback = |e: &str| {
        Response::build(res.status())
            .content_type("text/plain")
            .body(e.to_string())
    };

    let tmpl = request.app_data::<web::Data<Tera>>().map(|t| t.get_ref());
    let config = request
        .app_data::<web::Data<config::WebSiteConfig>>()
        .map(|t| t.get_ref());
    match tmpl {
        Some(tmpl) => {
            let mut context = tera::Context::new();
            context.insert("error", error);
            context.insert("status_code", res.status().as_str());
            context.insert("CONFIG", &config);

            let body = tmpl.render("error.html", &context);
            match body {
                Ok(body) => Response::build(res.status())
                    .content_type("text/html")
                    .body(body),
                Err(_) => fallback(error),
            }
        }
        None => fallback(error),
    }
}
