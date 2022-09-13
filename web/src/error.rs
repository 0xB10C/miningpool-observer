use miningpool_observer_shared::config;
use tera::Tera;

use actix_web::{
    body::BoxBody, dev::ServiceResponse, http::header::ContentType,
    middleware::ErrorHandlerResponse, web, HttpResponse, Result,
};

pub fn template_error(e: tera::Error) -> actix_web::Error {
    log::error!("Template Error: {}", e);
    actix_web::error::ErrorInternalServerError("Template Error")
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
    match tmpl {
        Some(tmpl) => {
            let mut context = tera::Context::new();
            context.insert("error", error);
            context.insert("status_code", res.status().as_str());
            context.insert("CONFIG", &config);

            let body = tmpl.render("error.html", &context);
            match body {
                Ok(body) => HttpResponse::build(res.status())
                    .content_type(ContentType::html())
                    .body(body),
                Err(_) => fallback(error),
            }
        }
        None => fallback(error),
    }
}
