use axum::{
    body::Body,
    extract::Request,
    http::{Method, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};

pub async fn serve(request: Request) -> Response {
    if !matches!(request.method(), &Method::GET | &Method::HEAD) {
        return super::not_found().await.into_response();
    }
    let path = request.uri().path();
    if path == "/" {
        return Redirect::temporary("console/index.html").into_response();
    }
    if path == "/console" {
        return Redirect::temporary("console/index.html").into_response();
    }
    if path == "/console/" {
        return Redirect::temporary("index.html").into_response();
    }
    let Some(path) = path.strip_prefix("/console/") else {
        return super::not_found().await.into_response();
    };
    let Some(asset) = hps_web_assets::get(path) else {
        return super::not_found().await.into_response();
    };
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, hps_web_assets::content_type(path))
        .header(header::CONTENT_LENGTH, asset.len())
        .header(header::CACHE_CONTROL, cache)
        .header("x-content-type-options", "nosniff")
        .header("x-hps-web-build-id", hps_web_assets::BUILD_ID)
        .body(if request.method() == Method::HEAD {
            Body::empty()
        } else {
            Body::from(asset)
        })
        .expect("static asset response")
}
