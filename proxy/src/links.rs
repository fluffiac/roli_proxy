use std::collections::HashMap;
use std::sync::OnceLock;

use futures::{Future, FutureExt};
use itertools::Itertools;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
use tokio::sync::{broadcast, mpsc};

use crate::api;
use crate::image::{self, Image};
use crate::promise::{LazyPromise, Promise};
use crate::query::{Query, QueryBuilder};

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
        LinkMap::get_lock().read().await
    }

    async fn get_mut_ref() -> MutRef<Self> {
        LinkMap::get_lock().write().await
    }

    pub fn get(&self, id: usize) -> Option<Link> {
        self.inner.get(&id).cloned()
    }

    pub fn insert_image(&mut self, id: (usize, usize), res: (LazyPromise<Option<Image>>, Refresher)) {
        println!("inserting image: {:?}", id);

        self.inner.insert(id.0, Link::Image(res.0));
        self.inner.insert(id.1, Link::RefreshImage(res.1));
    }

    pub fn remove_image(&mut self, id: (usize, usize)) {
        println!("removing image: {:?}", id);

        self.inner.remove(&id.0);
        self.inner.remove(&id.1);
    }

    pub fn insert_preview(&mut self, id: usize, res: Promise<Option<Image>>) {
        println!("inserting preview: {:?}", id);

        self.inner.insert(id, Link::Previews(res));
    }

    pub fn remove_preview(&mut self, id: usize) {
        println!("removing preview: {:?}", id);

        self.inner.remove(&id);
    }

    pub fn insert_query(&mut self, id: (usize, usize), res: (Query, Refresher)) {
        println!("inserting query: {:?}", id);

        self.inner.insert(id.0, Link::Query(res.0));
        self.inner.insert(id.1, Link::RefreshQuery(res.1));
    }

    pub fn remove_query(&mut self, id: (usize, usize)) {
        println!("removing query: {:?}", id);

        self.inner.remove(&id.0);
        self.inner.remove(&id.1);
    }
}

#[derive(Clone)]
pub enum Link {
    /// (lazy image data)
    Previews(Promise<Option<Image>>),
    /// (url, lazy image data)
    Image(LazyPromise<Option<Image>>),
    /// (query)
    Query(Query),
    /// (image refresher)
    RefreshImage(Refresher),
    /// (Query, refresher)
    RefreshQuery(Refresher),
}

struct Ids {
    pub post_ids: Vec<(usize, usize)>,
    pub query_id: usize,
    pub preview_id: usize,
    pub refresh_id: usize,
}

impl Ids {
    /// Aquire a set of unique ids to associate with the Query values
    fn new(links: &LinkMap, post_ct: usize) -> Self {
        let mut ids = (0_usize..).filter(|k| !links.inner.contains_key(k));

        let post_ids = ids.by_ref().take(post_ct * 2).tuples().collect();

        Self {
            post_ids,
            query_id: ids.next().expect("never ending iterator ended"),
            preview_id: ids.next().expect("never ending iterator ended"),
            refresh_id: ids.next().expect("never ending iterator ended"),
        }
    }
}

#[derive(Clone)]
pub enum Refresher {
    One(mpsc::Sender<()>),
    Many(broadcast::Sender<()>)
}

impl Refresher {
    pub fn refresh(&self) {
        match self {
            Self::One(tx) => drop(tx.try_send(())),
            Self::Many(tx) => drop(tx.send(())),
        }
    }
}

struct RefreshHandler {
    refresh: broadcast::Sender<()>,
}

impl RefreshHandler {
    fn new() -> Self {
        Self { 
            refresh: broadcast::channel(1).0   
        }
    }

    /// Attach a teardown future to this handlers global refresh signal.
    ///
    /// The execution of the teardown future will begin after the given
    /// duration. Calling the `refresh` method on a `Refresher` associated 
    /// with this handler will reset the timer.
    fn attach<F>(&self, len: u64, f: F)
    where
        F: Future + Send + 'static
    {
        let mut many = self.refresh.subscribe();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = sleep(Duration::from_secs(len)) => break,
                    Ok(()) = many.recv() => (),
                }
    
                println!("resource refreshed");
            }

            f.await;
        });
    }

    /// Attach a teardown future to this handlers global refresh signal,
    /// and a refresh signal local to this specific future.
    ///
    /// The execution of the teardown future will begin after the given
    /// duration. Calling the `refresh` method on a `Refresher` associated 
    /// with this handler, or the one returned by this method, will reset
    /// the timer.
    fn attach_with_local<F>(&self, len: u64, f: F) -> Refresher
    where
        F: Future + Send + 'static,
    {
        let mut many = self.refresh.subscribe();
        let (refresh, mut one) = mpsc::channel(1);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = sleep(Duration::from_secs(len)) => break,
                    Ok(()) = many.recv() => (),
                    Some(()) = one.recv() => (),
                }
    
                println!("resource refreshed");
            }

            f.await;
        });

        Refresher::One(refresh)
    }

    fn into_refresher(self) -> Refresher {
        Refresher::Many(self.refresh)
    }
}

// todo: refactor into BIG FAT FUNCTION
pub struct NewLinkCtx {
    map: MutRef<LinkMap>,
    refresh: RefreshHandler,
    posts: api::Posts,
    ids: Ids,
}

impl NewLinkCtx {
    async fn new(posts: api::Posts) -> Self {
        let map = LinkMap::get_mut_ref().await;
        let ids = Ids::new(&map, posts.len());

        Self {
            map,
            refresh: RefreshHandler::new(),
            ids,
            posts,
        }
    }
}

fn setup_posts(ctx: &mut NewLinkCtx) {
    for (post, &ids) in ctx.posts.iter().zip(&ctx.ids.post_ids) {
        let url = post.sample.url.clone();

        let image = LazyPromise::new(async move {
            println!("fetching image: {url}");
            api::get_image(url).map(Result::ok).await
        });

        let refresh = ctx.refresh.attach_with_local(1200, async move {
            let mut map = LinkMap::get_mut_ref().await;

            map.remove_image(ids);
        });

        ctx.map.insert_image(ids, (image, refresh));
    }
}

async fn setup_preview(ctx: &mut NewLinkCtx) {
    let id = ctx.ids.preview_id;

    let preview = Promise::new(image::make_preview(ctx.posts.clone())).await;

    ctx.refresh.attach(600, async move {
        let mut map = LinkMap::get_mut_ref().await;

        map.remove_preview(id);
    });

    ctx.map.insert_preview(id, preview);
}

fn setup_query(mut ctx: NewLinkCtx) -> Query {
    let query = new_query(&ctx);
    let ids = (ctx.ids.query_id, ctx.ids.refresh_id);

    ctx.refresh.attach(600, async move {
        let mut map = LinkMap::get_mut_ref().await;

        map.remove_query(ids);
    });

    let refresher = ctx.refresh.into_refresher();
    ctx.map.insert_query(ids, (query.clone(), refresher));

    query
}

fn new_query(ctx: &NewLinkCtx) -> Query {
    let mut builder = QueryBuilder::new();
    builder.push_header(ctx.ids.query_id, ctx.ids.preview_id, ctx.ids.refresh_id);

    for (post, &ids) in ctx.posts.iter().zip(&ctx.ids.post_ids) {
        builder.push_post(post, ids);
    }

    builder.into_query()
}

pub async fn setup_links(posts: api::Posts) -> Query {
    let mut ctx = NewLinkCtx::new(posts).await;

    setup_posts(&mut ctx);
    setup_preview(&mut ctx).await;
    setup_query(ctx)
}