//! Contains a `LinkMap` struct that maps identifiers to `Link` variants.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use futures::FutureExt;
use itertools::Itertools;
use tokio::sync::RwLock;

use crate::api;
use crate::image::{self, Image};
use crate::promise::{LazyPromise, Promise};
use crate::refresh::{RefreshHandler, Refresher};

/// A map of `Link` variants, with their associated identifiers.
///
/// This struct is used to manage the lifecycle of `Link` variants, which are
/// used to store information about resources that are available to the client.
/// When the client makes a request to the `/s/` endpoint, the server will
/// generate a `SearchMap` string that informs the client on how to fetch and
/// refresh the resources assocaited with the query that generated the
/// `SearchMap`.
///
/// The `setup_links` function is called by the server to generate the `links`
/// for a given API response. This initially inserts several `Link` variants,
/// and spawns tasks that will remove them after a certain period of time.
/// The task details can be found in the `RefreshHandler` struct.
#[derive(Default)]
pub struct LinkMap {
    inner: HashMap<usize, Link>,
}

/// A map of `Link` variants.
///
/// See `LinkMap` for information on the lifecycle for values of this type,
/// or the `link` function for information on the specific variants of `Link`.
#[derive(Clone)]
pub enum Link {
    /// Preview image `Promise`
    Previews(Promise<Option<Image>>),
    /// Sample image `LazyPromise`
    Image(LazyPromise<Option<Image>>),
    /// (search query)
    SearchMap(SearchMap),
    /// (image refresher)
    RefreshImage(Refresher),
    /// (Query, refresher)
    RefreshSearch(Refresher),
}

/// A reference to a `LinkMap`.
type Ref<T> = tokio::sync::RwLockReadGuard<'static, T>;
/// A mutable reference to a `LinkMap`.
type MutRef<T> = tokio::sync::RwLockWriteGuard<'static, T>;

impl LinkMap {
    /// Get a lock to the global `LinkMap`.
    fn get_lock() -> &'static RwLock<Self> {
        static MAP: OnceLock<RwLock<LinkMap>> = OnceLock::new();
        MAP.get_or_init(Default::default)
    }

    /// Get a reference to the global `LinkMap`.
    pub async fn get_ref() -> Ref<Self> {
        Self::get_lock().read().await
    }

    /// Get a mutable reference to the global `LinkMap`.
    async fn get_mut_ref() -> MutRef<Self> {
        Self::get_lock().write().await
    }

    /// Get an `Link` variant from its identifier, if it exists.
    pub fn get(&self, id: usize) -> Option<Link> {
        self.inner.get(&id).cloned()
    }

    /// Get a list of free identifiers that can be used to insert new `Link`
    /// variants.
    fn get_free_ids(&self, posts: &api::Posts) -> (Vec<(api::Post, PostIds)>, HeaderIds) {
        let mut ids = (0_usize..).filter(|k| !self.inner.contains_key(k));

        let id_tups = ids
            .by_ref()
            .take(posts.len() * 2)
            .tuples()
            .map(PostIds::new);
        let post_ids = posts.iter().cloned().zip(id_tups).collect();

        let query_ids = HeaderIds {
            search_map: ids.next().expect("never ending iter ended"),
            preview: ids.next().expect("never ending iter ended"),
            refresh: ids.next().expect("never ending iter ended"),
        };

        (post_ids, query_ids)
    }

    /// Insert an image `Link` into the map.
    fn insert_image(&mut self, ids: PostIds, res: (LazyPromise<Option<Image>>, Refresher)) {
        log::info!("inserting image: {}", ids.post);

        self.inner.insert(ids.post, Link::Image(res.0));
        self.inner.insert(ids.refresh, Link::RefreshImage(res.1));
    }

    /// Remove an image `Link` from the map.
    ///
    /// This is called by the `RefreshHandler` after a certain period of time,
    /// unless a client calls its associated refresher `link`.
    fn remove_image(&mut self, ids: PostIds) {
        log::info!("removing image: {}", ids.post);

        self.inner.remove(&ids.post);
        self.inner.remove(&ids.refresh);
    }

    /// Insert a preview `Link` into the map.
    fn insert_preview(&mut self, ids: HeaderIds, res: Promise<Option<Image>>) {
        log::info!("inserting preview: {}", ids.preview);

        self.inner.insert(ids.preview, Link::Previews(res));
    }

    /// Remove a preview `Link` from the map.
    ///
    /// This is called by the `RefreshHandler` after a certain period of time,
    /// unless a client calls its associated refresher `link`.
    fn remove_preview(&mut self, ids: HeaderIds) {
        log::info!("removing preview: {}", ids.preview);

        self.inner.remove(&ids.preview);
    }

    /// Insert a `SearchMap` `Link` into the map.
    fn insert_query(&mut self, ids: HeaderIds, res: (SearchMap, Refresher)) {
        log::info!("inserting query: {}", ids.search_map);

        self.inner.insert(ids.search_map, Link::SearchMap(res.0));
        self.inner.insert(ids.refresh, Link::RefreshSearch(res.1));
    }

    /// Remove a `SearchMap` `Link` from the map.
    ///
    /// This is called by the `RefreshHandler` after a certain period of time,
    /// unless a client calls its associated refresher `link`.
    fn remove_query(&mut self, ids: HeaderIds) {
        log::info!("removing query: {}", ids.search_map);

        self.inner.remove(&ids.search_map);
        self.inner.remove(&ids.refresh);
    }
}

/// From a list of `Posts` returned from the e621 API, create a `SearchMap`
/// string that informs clients on how to fetch the posts returned by their
/// search query.
pub async fn setup_links(posts: api::Posts) -> SearchMap {
    // obtain a mut LinkMap ref by locking the global struct.
    let mut map = LinkMap::get_mut_ref().await;

    let refresh_handler = RefreshHandler::new();
    let (post_ids, header_ids) = map.get_free_ids(&posts);

    let mut builder = SeachMapBuilder::new_with_header(header_ids);

    for (post, ids) in post_ids {
        builder.push_post(&post, ids);

        let refresher = refresh_handler.attach_with_local(1200, async move {
            LinkMap::get_mut_ref().await.remove_image(ids);
        });

        let url = post.sample.url.clone();
        let image = LazyPromise::new(api::get_image(url).map(Result::ok));

        map.insert_image(ids, (image, refresher));
    }

    let search_map = builder.into_query();
    let preview = Promise::new(image::make_preview(posts.clone())).await;

    refresh_handler.attach(600, async move {
        let mut map = LinkMap::get_mut_ref().await;

        map.remove_query(header_ids);
        map.remove_preview(header_ids);
    });

    map.insert_preview(header_ids, preview);
    map.insert_query(
        header_ids,
        (search_map.clone(), refresh_handler.into_refresher()),
    );

    search_map
}

/// Helper struct that names the identifiers for a `SearchMap` header.
#[derive(Clone, Copy)]
struct HeaderIds {
    search_map: usize,
    preview: usize,
    refresh: usize,
}

/// Helper struct that names the identifiers for a `SearchMap` post.
#[derive(Clone, Copy)]
struct PostIds {
    post: usize,
    refresh: usize,
}

impl PostIds {
    /// Create a new `PostIds` from a pair of identifiers.
    const fn new(ids: (usize, usize)) -> Self {
        Self {
            post: ids.0,
            refresh: ids.1,
        }
    }
}

/// A `SearchMap` string.
///
/// This is a type alias for an `Arc<str>`, which is a reference-counted string.
/// This allows for the `SearchMap` to be shared between multiple threads
/// without needing to clone the inner data.
type SearchMap = Arc<str>;

/// A builder for creating a `SearchMap` string.
///
/// This builder is a helper for creating the string returned by the `e.roli.ga`
/// `/s/` endpoint. The format is described in the `new_with_header` and
/// `push_post` methods.
struct SeachMapBuilder(String);

impl SeachMapBuilder {
    /// Construct a new `SearchMapBuilder`.
    ///
    /// This function builds the headers for the `SearchMap` string.
    fn new_with_header(ids: HeaderIds) -> Self {
        let mut this = Self(String::new());
        this.push_element::<' '>("600000")
            .push_element::<','>(&ids.search_map.to_string())
            .push_element::<','>(&ids.preview.to_string())
            .push_element::<','>(&ids.refresh.to_string());
        this
    }

    /// Push `Post` metadata to the inner  `SearchMap` string, along with it's
    /// `link` ids.
    fn push_post(&mut self, post: &api::Post, ids: PostIds) -> &mut Self {
        self.push_element::<'\n'>(&ids.post.to_string())
            .push_element::<','>(&post.id.to_string())
            .push_element::<','>(&post.sample.width.to_string())
            .push_element::<','>(&post.sample.height.to_string())
            .push_element::<','>(&post.preview.width.to_string())
            .push_element::<','>(&post.preview.height.to_string())
            .push_element::<','>(&post.score.up.to_string())
            .push_element::<','>(&post.score.down.to_string())
            .push_element::<','>(&post.rating)
            .push_element::<','>(&post.file.ext)
            .push_element::<','>(&ids.refresh.to_string())
            .push_element::<','>("1200000")
    }

    /// Push an element to the inner `SearchMap` string.
    fn push_element<const SEPARATOR: char>(&mut self, element: &str) -> &mut Self {
        match SEPARATOR {
            ',' => self.0.push(','),
            '\n' => self.0.push('\n'),
            ' ' => (),
            _ => unreachable!(),
        }
        self.0.push_str(element);
        self
    }

    /// Convert the `SearchMapBuilder` into a `SearchMap`.
    fn into_query(self) -> SearchMap {
        Arc::from(self.0.into_boxed_str())
    }
}
