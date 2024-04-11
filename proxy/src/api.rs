use std::sync::{Arc, OnceLock};

use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::Value;

struct Client {
    client: &'static reqwest::Client,
}

impl Client {
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
                .unwrap()
        });

        Self { client }
    }

    async fn get(&self, url: &str) -> Result<reqwest::Response, reqwest::Error> {
        self.client.get(url).send().await
    }
}

#[derive(Clone)]
pub struct Image {
    pub data: Box<[u8]>,
    pub mime_type: Arc<str>,
}

impl Image {
    pub fn new(data: Box<[u8]>, mime_type: Arc<str>) -> Self {
        Self { data, mime_type }
    }
}

impl IntoResponse for Image {
    fn into_response(self) -> Response {
        (
            [(header::CONTENT_TYPE, self.mime_type.to_string())],
            self.data,
        )
            .into_response()
    }
}

// todo: would returning the content type be useful?
pub async fn image(url: String) -> Result<Image, reqwest::Error> {
    let res = Client::global().get(&url).await?;

    let mime_type = res
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream");
    let mime_type = Arc::from(mime_type);

    let data = res.bytes().await?.to_vec().into_boxed_slice();

    Ok(Image::new(data, mime_type))
}

// blacklist
const EXCLUDES: &str = "-young+-type:webm+-type:gif";

pub async fn query(query: &str, page: &str) -> Result<Posts, reqwest::Error> {
    let url = format!("https://e621.net/posts.json?limit=20&page={page}&tags={query}+{EXCLUDES}");

    Client::global().get(&url).await?.json().await
}

//////////////////////////////////////////////////////////
// auto-generated code

// todo: use Arc<str> or something cheaper than String

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Posts {
    pub posts: Vec<Post>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    pub id: i64,
    #[serde(rename = "created_at")]
    pub created_at: String,
    #[serde(rename = "updated_at")]
    pub updated_at: String,
    pub file: File,
    pub preview: Preview,
    pub sample: Sample,
    pub score: Score,
    pub tags: Tags,
    #[serde(rename = "locked_tags")]
    pub locked_tags: Vec<String>,
    #[serde(rename = "change_seq")]
    pub change_seq: i64,
    pub flags: Flags,
    pub rating: String,
    #[serde(rename = "fav_count")]
    pub fav_count: i64,
    pub sources: Vec<String>,
    pub pools: Vec<i64>,
    pub relationships: Relationships,
    #[serde(rename = "approver_id")]
    pub approver_id: Option<i64>,
    #[serde(rename = "uploader_id")]
    pub uploader_id: i64,
    pub description: String,
    #[serde(rename = "comment_count")]
    pub comment_count: i64,
    #[serde(rename = "is_favorited")]
    pub is_favorited: bool,
    #[serde(rename = "has_notes")]
    pub has_notes: bool,
    pub duration: Value,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct File {
    pub width: i64,
    pub height: i64,
    pub ext: String,
    pub size: i64,
    pub md5: String,
    pub url: String,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub width: i64,
    pub height: i64,
    pub url: String,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub has: bool,
    pub height: i64,
    pub width: i64,
    pub url: String,
    pub alternates: Alternates,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alternates {}

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Score {
    pub up: i64,
    pub down: i64,
    pub total: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tags {
    pub general: Vec<String>,
    pub artist: Vec<String>,
    pub copyright: Vec<String>,
    pub character: Vec<String>,
    pub species: Vec<String>,
    pub invalid: Vec<String>,
    pub meta: Vec<String>,
    pub lore: Vec<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct Flags {
    pub pending: bool,
    pub flagged: bool,
    #[serde(rename = "note_locked")]
    pub note_locked: bool,
    #[serde(rename = "status_locked")]
    pub status_locked: bool,
    #[serde(rename = "rating_locked")]
    pub rating_locked: bool,
    pub deleted: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Relationships {
    #[serde(rename = "parent_id")]
    pub parent_id: Option<i64>,
    #[serde(rename = "has_children")]
    pub has_children: bool,
    #[serde(rename = "has_active_children")]
    pub has_active_children: bool,
    pub children: Vec<i64>,
}
