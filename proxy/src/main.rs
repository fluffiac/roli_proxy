#![forbid(unsafe_code)]
// #![warn(clippy::missing_docs_in_private_items)]

use std::io;
use std::{net::SocketAddr, path::PathBuf};

use axum::extract::{Path, Request};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;

use crate::image::Image;
use crate::links::{setup_links, Link, LinkMap};

mod api;
mod image;
mod links;
mod promise;
mod query;

//////////////////////////////////////////////////////////
// main

#[tokio::main]
async fn main() -> io::Result<()> {
    let app = Router::new()
        .route("/proxy_status", get(|| async { text("proxy OK") }))
        .route("/status", get(|| async { text("OK") }))
        .route("/link/:id", get(link))
        .route("/s/", get(|| query(Path(String::new()))))
        .route("/s/:query", get(query))
        .fallback(fallback);

    let config = RustlsConfig::from_pem_file(
        PathBuf::from("./").join("https_certs").join("server.crt"),
        PathBuf::from("./").join("https_certs").join("server.key"),
    )
    .await
    .map_err(io::Error::other)?;

    let addr = SocketAddr::from(([0, 0, 0, 0], 443));
    println!("listening on {addr}");
    axum_server::bind_rustls(addr, config)
        .serve(app.into_make_service())
        .await
}

//////////////////////////////////////////////////////////
// helper fns

fn text(str: impl Into<String>) -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        str.into(),
    )
        .into_response()
}

fn missing() -> Image {
    let bytes = include_bytes!("../../assets/missing.png");

    Image::new(bytes.to_vec().into_boxed_slice(), "image/png".into())
}

//////////////////////////////////////////////////////////
// handlers

async fn fallback(req: Request) -> Response {
    println!("### fallback ######################");
    println!("tried to GET: {}", req.uri().path());
    println!("###################################");

    text(format!(
        r#"<html lang="en">
    <head>
        <meta charset="utf-8">
        <title>Error</title>
    </head>
    <body>
        <pre>Cannot GET {}</pre>
    </body>
</html>"#,
        req.uri().path()
    ))
}

async fn link(Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse() else {
        return text("Link expired");
    };

    let Some(link) = LinkMap::get_ref().await.get(id) else {
        return text("Link expired");
    };

    match link {
        Link::Previews(image) => {
            println!("get previews: {id}");
            image
                .get()
                .await
                .clone()
                .unwrap_or_else(missing)
                .into_response()
        }
        Link::Image(image) => {
            println!("get image: {id}");
            image
                .get()
                .await
                .clone()
                .unwrap_or_else(missing)
                .into_response()
        }
        Link::Query(query) => {
            println!("get query: {id}");
            text(query.to_string())
        }
        Link::RefreshImage(refresh) => {
            println!("refreshing image: {id}");
            refresh.refresh();
            text("1200000")
        }
        Link::RefreshQuery(refresh) => {
            println!("refreshing query: {id}");
            refresh.refresh();
            text("600000")
        }
    }
}

async fn query(Path(query): Path<String>) -> Response {
    let mut query = query.trim();
    let mut page = "1";

    // if the last thing is a number, it's a page
    if let Some(tpage) = query.split_whitespace().last() {
        if tpage.parse::<usize>().is_ok() {
            query = &query[..query.len() - page.len()];
            page = tpage;
        }
    }

    println!("query: {query} page {page}");
    let Ok(posts) = api::query(query, page).await else {
        return text("An error occured during the external query.");
    };

    text(setup_links(posts).await.to_string())
}
