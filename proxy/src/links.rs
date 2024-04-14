use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use futures::FutureExt;
use itertools::Itertools;
use tokio::sync::RwLock;

use crate::api;
use crate::image::{self, Image};
use crate::promise::{LazyPromise, Promise};
use crate::refresh::{RefreshHandler, Refresher};

#[derive(Clone)]
pub enum Link {
    /// Preview image `Promise`
    Previews(Promise<Option<Image>>),
    /// Sample image `LazyPromise`
    Image(LazyPromise<Option<Image>>),
    /// (query)
    Query(Query),
    /// (image refresher)
    RefreshImage(Refresher),
    /// (Query, refresher)
    RefreshQuery(Refresher),
}

#[derive(Default)]
pub struct LinkMap {
    inner: HashMap<usize, Link>,
}

type Ref<T> = tokio::sync::RwLockReadGuard<'static, T>;
type MutRef<T> = tokio::sync::RwLockWriteGuard<'static, T>;

impl LinkMap {
    fn get_lock() -> &'static RwLock<Self> {
        static MAP: OnceLock<RwLock<LinkMap>> = OnceLock::new();
        MAP.get_or_init(Default::default)
    }

    pub async fn get_ref() -> Ref<Self> {
        Self::get_lock().read().await
    }

    async fn get_mut_ref() -> MutRef<Self> {
        Self::get_lock().write().await
    }

    pub fn get(&self, id: usize) -> Option<Link> {
        self.inner.get(&id).cloned()
    }

    pub fn get_free_ids(&self, posts: &api::Posts) -> (Vec<(api::Post, PostIds)>, QueryIds) {
        let mut ids = (0_usize..).filter(|k| !self.inner.contains_key(k));

        let id_tups = ids
            .by_ref()
            .take(posts.len() * 2)
            .tuples()
            .map(PostIds::new);
        let post_ids = posts.iter().cloned().zip(id_tups).collect();

        let query_ids = QueryIds {
            query: ids.next().expect("never ending iter ended"),
            preview: ids.next().expect("never ending iter ended"),
            refresh: ids.next().expect("never ending iter ended"),
        };

        (post_ids, query_ids)
    }

    pub fn insert_image(&mut self, ids: PostIds, res: (LazyPromise<Option<Image>>, Refresher)) {
        log::info!("inserting image: {}", ids.post);

        self.inner.insert(ids.post, Link::Image(res.0));
        self.inner.insert(ids.refresh, Link::RefreshImage(res.1));
    }

    pub fn remove_image(&mut self, ids: PostIds) {
        log::info!("removing image: {}", ids.post);

        self.inner.remove(&ids.post);
        self.inner.remove(&ids.refresh);
    }

    pub fn insert_preview(&mut self, ids: QueryIds, res: Promise<Option<Image>>) {
        log::info!("inserting preview: {}", ids.preview);

        self.inner.insert(ids.preview, Link::Previews(res));
    }

    pub fn remove_preview(&mut self, ids: QueryIds) {
        log::info!("removing preview: {}", ids.preview);

        self.inner.remove(&ids.preview);
    }

    pub fn insert_query(&mut self, ids: QueryIds, res: (Query, Refresher)) {
        log::info!("inserting query: {}", ids.query);

        self.inner.insert(ids.query, Link::Query(res.0));
        self.inner.insert(ids.refresh, Link::RefreshQuery(res.1));
    }

    pub fn remove_query(&mut self, ids: QueryIds) {
        log::info!("removing query: {}", ids.query);

        self.inner.remove(&ids.query);
        self.inner.remove(&ids.refresh);
    }
}

pub async fn setup_links(posts: api::Posts) -> Query {
    let mut map = LinkMap::get_mut_ref().await;

    let refresh = RefreshHandler::new();
    let (post_ids, ids) = map.get_free_ids(&posts);

    let mut builder = QueryBuilder::new();
    builder.push_header(ids);

    for (post, ids) in post_ids {
        builder.push_post(&post, ids);

        let refresh = refresh.attach_with_local(1200, async move {
            LinkMap::get_mut_ref().await.remove_image(ids);
        });

        let url = post.sample.url.clone();
        let image = LazyPromise::new(api::get_image(url).map(Result::ok));

        map.insert_image(ids, (image, refresh));
    }

    refresh.attach(600, async move {
        let mut map = LinkMap::get_mut_ref().await;

        map.remove_query(ids);
        map.remove_preview(ids);
    });

    let query = builder.into_query();
    let preview = Promise::new(image::make_preview(posts.clone())).await;

    map.insert_query(ids, (query.clone(), refresh.into_refresher()));
    map.insert_preview(ids, preview);

    query
}

#[derive(Clone, Copy)]
pub struct QueryIds {
    pub query: usize,
    pub preview: usize,
    pub refresh: usize,
}

#[derive(Clone, Copy)]
pub struct PostIds {
    pub post: usize,
    pub refresh: usize,
}

impl PostIds {
    const fn new(ids: (usize, usize)) -> Self {
        Self {
            post: ids.0,
            refresh: ids.1,
        }
    }
}

pub type Query = Arc<str>;

pub struct QueryBuilder(String);

impl QueryBuilder {
    pub const fn new() -> Self {
        Self(String::new())
    }

    pub fn push_header(&mut self, ids: QueryIds) -> &mut Self {
        self.push_element("600000")
            .push_element(&ids.query.to_string())
            .push_element(&ids.preview.to_string())
            .push_element(&ids.refresh.to_string())
    }

    pub fn push_post(&mut self, post: &api::Post, ids: PostIds) -> &mut Self {
        self.push_newline()
            .push_element(&ids.post.to_string())
            .push_element(&post.id.to_string())
            .push_element(&post.sample.width.to_string())
            .push_element(&post.sample.height.to_string())
            .push_element(&post.preview.width.to_string())
            .push_element(&post.preview.height.to_string())
            .push_element(&post.score.up.to_string())
            .push_element(&post.score.down.to_string())
            .push_element(&post.rating)
            .push_element(&post.file.ext)
            .push_element(&ids.refresh.to_string())
            .push_element("1200000")
    }

    pub fn push_element(&mut self, element: &str) -> &mut Self {
        if let None | Some('\n') = self.0.chars().last() {
        } else {
            self.0.push(',');
        }
        self.0.push_str(element);
        self
    }

    pub fn push_newline(&mut self) -> &mut Self {
        self.0.push('\n');
        self
    }

    pub fn into_query(self) -> Query {
        Arc::from(self.0.into_boxed_str())
    }
}
