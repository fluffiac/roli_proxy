//! Manage backend API requests and responses.

use std::sync::{Arc, OnceLock};

use crate::image::Image;

/// Hardcoded blacklist
const EXCLUDES: &str = "-young";

/// Query the e621 API with a given query string and page number.
pub async fn query(query: &str, page: &str) -> Result<Posts, reqwest::Error> {
    let url = format!("https://e621.net/posts.json?limit=20&page={page}&tags={query}+{EXCLUDES}+-type:webm+-type:gif");

    let posts: Root = HttpClient::global().get(&url).await?.json().await?;

    Ok(posts.posts)
}

/// Get an image from a URL, and return it as the crate `Image` type.
pub async fn get_image(url: Arc<str>) -> Result<Image, reqwest::Error> {
    log::info!("getting image: {url}");

    let res = HttpClient::global().get(&url).await?;

    let mime_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream");
    let mime_type = Arc::from(mime_type);

    let data = res.bytes().await?.to_vec().into_boxed_slice();

    Ok(Image::new(data, mime_type))
}

/// An HTTP client for the e621 API, with authorization headers.
struct HttpClient {
    client: &'static reqwest::Client,
}

impl HttpClient {
    /// Get a global instance of the http client.
    fn global() -> Self {
        static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

        let client = CLIENT.get_or_init(|| {
            let mut headers = reqwest::header::HeaderMap::new();

            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_static(env!("E6AUTH")),
            );

            reqwest::Client::builder()
                .user_agent("e6proxy/0.0 (by fluffiac :3)")
                .default_headers(headers)
                .build()
                .expect("valid headers are invalid")
        });

        Self { client }
    }

    /// Perform a GET request.
    async fn get(&self, url: &str) -> Result<reqwest::Response, reqwest::Error> {
        self.client.get(url).send().await
    }
}

//////////////////////////////////////////////////////////
// JSON structure

// used to deserialize the json response, immediately turned into `Posts`
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[doc(hidden)]
struct Root {
    posts: Arc<[Post]>,
}

/// A list of posts from the e621 API.
pub type Posts = Arc<[Post]>;

/// A post from the e621 API.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    pub id: u64,
    pub file: File,
    pub preview: Preview,
    pub sample: Sample,
    pub score: Score,
    pub rating: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub width: i64,
    pub height: i64,
    pub ext: Arc<str>,
    pub size: i64,
    pub md5: Arc<str>,
    pub url: Arc<str>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub width: i64,
    pub height: i64,
    pub url: Arc<str>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub has: bool,
    pub height: i64,
    pub width: i64,
    pub url: Arc<str>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Score {
    pub up: i64,
    pub down: i64,
}
