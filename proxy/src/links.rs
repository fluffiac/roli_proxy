use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;

use futures::FutureExt;
use image::GenericImage;
use tokio::sync::RwLock;
use tokio::sync::{broadcast, mpsc};

use crate::api;
use crate::promise::{LazyPromise, Promise};
use crate::query::{QueryBuilder, Query};

static MAP: OnceLock<RwLock<HashMap<usize, Link>>> = OnceLock::new();

#[derive(Clone)]
pub enum Link {
    /// (lazy image data)
    Previews(Promise<Option<api::Image>>),
    /// (url, lazy image data)
    Image(LazyPromise<Option<api::Image>>),
    /// (query)
    Query(Query),
    /// (image refresher)
    RefreshImage(mpsc::Sender<()>),
    /// (Query, refresher)
    RefreshQuery(broadcast::Sender<()>),
}

type LinksRead = tokio::sync::RwLockReadGuard<'static, HashMap<usize, Link>>;
type LinksWrite = tokio::sync::RwLockWriteGuard<'static, HashMap<usize, Link>>;

pub async fn read_link_map() -> LinksRead {
    MAP.get_or_init(Default::default).read().await
}

pub async fn write_link_map() -> LinksWrite {
    MAP.get_or_init(Default::default).write().await
}

// [image, image_refresh], reget, previews, query_refresh
type Ids = (Vec<(usize, usize)>, usize, usize, usize);

// refactor into a better builder pattern with a ref instead of owned vec
pub struct LinkBuilder {
    map_ref: tokio::sync::RwLockWriteGuard<'static, HashMap<usize, Link>>,
    query_refresh: broadcast::Sender<()>,
    ids: Ids,
    posts: Vec<Arc<api::Post>>,
}

impl LinkBuilder {
    /// Aquire a set of unique ids to associate with the Query values
    fn get_ids(map_ref: &LinksWrite, post_ct: usize) -> Ids {
        let mut ids = (0..)
            .filter(|k| !map_ref.contains_key(k))
            .take(post_ct * 2 + 3)
            .collect::<Vec<_>>();

        let [query_id, preview_id, refresh_id] = ids.split_off(ids.len() - 3)[..] else {
            unreachable!()
        };

        let post_id = ids[..post_ct].iter().copied();
        let refresh_post = ids[post_ct..].iter().copied();

        (
            post_id.zip(refresh_post).collect(),
            query_id,
            preview_id,
            refresh_id,
        )
    }

    pub async fn new_query(posts: api::Posts) -> Query {
        let posts = posts
            .posts
            .iter()
            .cloned()
            .map(Arc::new)
            .collect::<Vec<_>>();

        Self::new(posts)
            .await
            .setup_posts()
            .setup_preview()
            .await
            .setup_query()
    }

    async fn new(posts: Vec<Arc<api::Post>>) -> Self {
        let (all_refresh, _) = broadcast::channel(1);

        let map_ref = write_link_map().await;

        let ids = Self::get_ids(&map_ref, posts.len());

        Self {
            map_ref,
            query_refresh: all_refresh,
            ids,
            posts,
        }
    }

    fn setup_posts(mut self) -> Self {
        let posts = self.posts.iter().cloned();
        let posts_ids = posts.zip(self.ids.0.iter());

        for (post, &(image_id, refresh_id)) in posts_ids {
            let (refresh_tx, refresh_rx) = mpsc::channel(1);
            let query_refresh = self.query_refresh.subscribe();

            // todo: Post Arc<str>
            let url = post.sample.url.clone();
            let promise = LazyPromise::new(async move {
                println!("fetching image: {url}");
                api::image(url).map(Result::ok).await
            });

            println!("registered: image {image_id}");
            println!("registered: image_refresh {refresh_id}");

            self.map_ref.insert(image_id, Link::Image(promise));
            self.map_ref
                .insert(refresh_id, Link::RefreshImage(refresh_tx));

            tokio::spawn(async move {
                expire(1200, query_refresh, Some(refresh_rx)).await;

                let mut map_ref = write_link_map().await;

                println!("expired: image {image_id}");
                println!("expired: image_refresh {refresh_id}");

                map_ref.remove(&image_id);
                map_ref.remove(&refresh_id);
            });
        }

        self
    }

    async fn setup_preview(mut self) -> Self {
        let (_, _, preview_id, _) = self.ids;

        let query_refresh = self.query_refresh.subscribe();

        // todo: Post Arc<str>
        let urls = self
            .posts
            .iter()
            .map(|post| post.preview.url.clone())
            .collect::<Vec<_>>();
        let promise = Promise::new(get_preview(urls)).await;

        println!("registered: preview {preview_id}",);

        self.map_ref.insert(preview_id, Link::Previews(promise));

        tokio::spawn(async move {
            expire(600, query_refresh, None).await;

            let mut map_ref = write_link_map().await;

            println!("expired: preview {preview_id}",);

            map_ref.remove(&preview_id);
        });

        self
    }

    fn setup_query(mut self) -> Query {
        let (post_ids, query_id, preview_id, refresh_id) = self.ids;
        let query_refresh = self.query_refresh.subscribe();

        let mut query = QueryBuilder::new();
        query.push_header(query_id, preview_id, refresh_id);

        let posts = self.posts.iter().zip(post_ids.iter());
        for (post, &(link_id, refresh_id)) in posts {
            query.push_post(post, link_id, refresh_id);
        }
        let query: Query = query.into_query();

        println!("registered: query {query_id}");
        println!("registered: query_refresh {refresh_id}");

        self.map_ref.insert(query_id, Link::Query(query.clone()));
        self.map_ref
            .insert(refresh_id, Link::RefreshQuery(self.query_refresh));

        tokio::spawn(async move {
            expire(600, query_refresh, None).await;

            let mut map_ref = write_link_map().await;

            println!("expired: query {query_id}");
            println!("expired: query_refresh {refresh_id}");

            map_ref.remove(&query_id);
            map_ref.remove(&refresh_id);
        });

        query
    }
}

// expiration logic
async fn expire(
    len: u64,
    mut renew_query: broadcast::Receiver<()>,
    renew_link: Option<mpsc::Receiver<()>>,
) {
    match renew_link {
        Some(mut link) => loop {
            tokio::select! {
                () = tokio::time::sleep(tokio::time::Duration::from_secs(len)) => break,
                Ok(()) = renew_query.recv() => (),
                Some(()) = link.recv() => (),
            }

            println!("resource refreshed");
        },
        None => loop {
            tokio::select! {
                () = tokio::time::sleep(tokio::time::Duration::from_secs(len)) => break,
                Ok(()) = renew_query.recv() => (),
            }

            println!("resource refreshed");
        },
    }
}

// todo: make not shitty
async fn get_preview(urls: Vec<String>) -> Option<api::Image> {
    println!("generating preview...");

    let urls = urls.iter().cloned().map(api::image);

    let pics = futures::future::try_join_all(urls).await.unwrap();

    let mut pic = image::ImageBuffer::<image::Rgba<u8>, _>::new(1500, 1500);

    for (i, img) in (0..pic.len()).zip(pics.into_iter()) {
        #[allow(clippy::cast_possible_truncation)]
        let i = i as u32;

        let mem = image::load_from_memory(&img.data).unwrap();

        let x = (i % 10) * 150 + (150 - mem.width()) / 2;
        let y = (i / 10) * 150 + (150 - mem.height()) / 2;

        pic.copy_from(&mem, x, y).unwrap();
    }

    let mut buf = std::io::Cursor::new(Vec::new());
    let _ = image::write_buffer_with_format(
        &mut buf,
        &pic,
        1500,
        1500,
        image::ExtendedColorType::Rgba8,
        image::ImageFormat::Png,
    );

    let vec = buf.into_inner();

    println!("finished generating preview");

    Some(api::Image::new(vec.into_boxed_slice(), "image/png".into()))
}
