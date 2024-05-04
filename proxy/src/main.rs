#![forbid(unsafe_code)]

//! # e.roli.ga Proxy Reimplementation
//! Rust reimplementation of the e.roli.ga e926 proxy.
//!
//! e.roli.ga is a proxy for the e926 furry art archival site, tailored to the
//! specific needs of the VRChat world scripting language. This project is a
//! reimplementation of that proxy, with some additional features. A user may
//! elect to route traffic bound for e.roli.ga to this version instead, either
//! by self-hosting or by using the public instance hosted at `3.22.67.226`.
//!
//! # Features
//!
//! - Explicit Results: This calls out to the e621 API instead of the e926 API.
//! - Pagination: The user may additionally specify a page number.
//!
//! # Client Lifecycle
//!
//! Clients interact with the proxy strictly through HTTP GET requests. The
//! first request a client should make is to `/s/`, here referred to as the
//! "search" endpoint. This endpoint calls the e621 API with the query string
//! provided by the user, and returns a `SearchMap` string after the proxy
//! finishes processing the API response. The processing includes fetching
//! "preview" images for each post in the response and stiching those together,
//! as well as caching "sample" image URLs, which the client may request later.
//! These items are referred to as "resources", which are accessed through the
//! `/link/` endpoint. The `SearchMap` response communicates the resources
//! available to the client, and additionally provides a "refresh link" for
//! each resource.

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

// utils
mod promise;
mod refresh;

// impl
mod api;
mod image;
mod links;

/// Program entry point.
#[tokio::main]
async fn main() -> io::Result<()> {
    JournalLog::new().unwrap().install().unwrap();
    log::set_max_level(LevelFilter::Info);

    let app = Router::new()
        .route("/check_jailbreak", get(|| async { text("jailbreak OK") }))
        .route("/status", get(|| async { text("OK") }))
        .route("/link/:id", get(link))
        .route("/s/", get(|| search(Path(String::new()))))
        .route("/s/:query", get(search))
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

/// Handler for the `/s/:query` endpoint.
/// 
/// See the crate documentation for more information on the client lifecycle.
async fn search(Path(query): Path<String>) -> Response {
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
/// This endpoint has multiple behaviors based on the kind of resource
/// associated with the id:
///
/// - `SearchMap`: Gets a SearchMap string. (Note: The SearchMap contains an ID
///              for itself. This is used to allow clients to display a search
///              even if they are not the ones that made it.
/// - `RefreshSearch`: Refreshes the SearchMap string.
/// - `Previews`: A stitched-together image of the preview images from the initial
///             search query.
/// - `Image`: The full-size image of a post from the initial search query.
/// - `RefreshImage`: Refreshes a full-size image resource.
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
        Link::SearchMap(sm) => {
            log::info!("get searchmap: {id}");
            text(sm.to_string())
        }
        Link::RefreshSearch(refresh) => {
            log::info!("refreshing searchmap: {id}");
            refresh.refresh();
            text("600000")
        }
        Link::Previews(image) => {
            log::info!("get previews: {id}");
            image
                .get()
                .await
                .clone()
                .unwrap_or_else(Image::placeholder)
                .into_response()
        }
        Link::Image(image) => {
            log::info!("get image: {id}");
            let image = image
                .get()
                .await
                .clone()
                .unwrap_or_else(Image::placeholder)
                .into_response();
            log::info!("serving image: {id}");
            image
        }
        Link::RefreshImage(refresh) => {
            log::info!("refreshing image: {id}");
            refresh.refresh();
            text("1200000")
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

/// Create a text/html response.
fn text(str: impl Into<String>) -> Response {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        str.into(),
    )
        .into_response()
}
