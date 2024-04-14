#![forbid(unsafe_code)]
// #![warn(clippy::missing_docs_in_private_items)]

//! # e.roli.ga Proxy Reimplementation
//! Rust reimplementation of the e.roli.ga e926 proxy.
//! 
//! The e.roli.ga e926 proxy is a server that acts as a middleman between a 
//! VRChat client and the e926 image board. Its design is oriented around the
//! specific limitations of the VRChat world scripting API. The VRChat client,
//! through the proxy, is able to deliver the expirience of browsing the image 
//! board.
//! 
//! # Proxy Design
//! 
//! A client first sends a query to the proxy at `/s/:query`. This returns a
//! string that the client parses to access resources associated with the query,
//! like images, a preview thumbnail, and keep-alive urls which prevent the
//! query or images from expiring.
//! 
//! # Additional Features
//! 
// todo flesh out docs

use std::io;
use std::{net::SocketAddr, path::PathBuf};

use axum::extract::{Path, Request};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use axum_server::tls_rustls::RustlsConfig;
use log::LevelFilter;
use systemd_journal_logger::JournalLog;

use crate::image::Image;
use crate::links::{setup_links, Link, LinkMap};

// specific implementation
mod api;
mod image;
mod links;

// generic utils
mod promise;
mod refresh;

//////////////////////////////////////////////////////////
// main

#[tokio::main]
async fn main() -> io::Result<()> {
    JournalLog::new().unwrap().install().unwrap();
    log::set_max_level(LevelFilter::Info);

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
    log::info!("listening on {addr}");
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

/// Handler for the `/s/:query` endpoint.
/// 
/// This is the first useful endpoint for a client. 
/// 
/// It sets up a list of 
/// `/link/:id` endpoints based on the result of an e621 query, and returns a
/// string that the client uses to access the resources associated with the 
/// query. These resources (stored in the global `LinkMap`) are kept alive by
/// the client through refreshing link requests. After a certain period time 
/// passes without a refresh, the resources are removed, and the id is freed 
/// for reuse.
// todo: document query features (such as page number and blacklists)
async fn query(Path(query): Path<String>) -> Response {
    // todo: add features to this query parsing, like pre-built blacklists
    let mut query = query.trim();
    let mut page = "1";

    // if the last thing is a number, it's a page
    if let Some(tpage) = query.split_whitespace().last() {
        if tpage.parse::<usize>().is_ok() {
            query = &query[..query.len() - page.len()];
            page = tpage;
        }
    }

    log::info!("query: {query} page {page}");
    let Ok(posts) = api::query(query, page).await else {
        return text("An error occured during the external query.");
    };

    text(setup_links(posts).await.to_string())
}

/// Handler for the `/link/:id` endpoint.
/// 
/// This endpoint returns resources initialized by a query call, which means 
/// this isn't useful unless you know how the links are organized (you need 
/// the `Query` string). A link may be a cached query string (for other clients
/// to understand the results of your query), a preview image, or a lazy-loaded
/// sample image. There are also refresh links, which the client can use to keep
/// the resources associated with its queries alive. 
async fn link(Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse() else {
        // mimics the behavior of the original proxy
        return text("Link expired");
    };

    let Some(link) = LinkMap::get_ref().await.get(id) else {
        // mimics the behavior of the original proxy
        return text("Link expired");
    };

    match link {
        Link::Previews(image) => {
            log::info!("get previews: {id}");
            image
                .get()
                .await
                .clone()
                .unwrap_or_else(missing)
                .into_response()
        }
        Link::Image(image) => {
            log::info!("get image: {id}");
            image
                .get()
                .await
                .clone()
                .unwrap_or_else(missing)
                .into_response()
        }
        Link::Query(query) => {
            log::info!("get query: {id}");
            text(query.to_string())
        }
        Link::RefreshImage(refresh) => {
            log::info!("refreshing image: {id}");
            refresh.refresh();
            text("1200000")
        }
        Link::RefreshQuery(refresh) => {
            log::info!("refreshing query: {id}");
            refresh.refresh();
            text("600000")
        }
    }
}

/// Handler for any route that doesn't match the other handlers.
/// 
/// Returns HTML to mimic the behavior of the original proxy.
async fn fallback(req: Request) -> Response {
    log::warn!("tried to GET: {}", req.uri().path());

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