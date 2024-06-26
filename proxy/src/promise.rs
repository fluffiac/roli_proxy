//! Promise types for asynchronusly computed values.
//!
//! The `Promise` and `LazyPromise` types are used to represent values that may
//! not be ready yet. The `Promise` type should be used when the computation
//! should start immediately, while the `LazyPromise` type should be used when
//! the computation should start only when the value is first "requested".

use std::future::Future;
use std::sync::Arc;

use futures::future::BoxFuture;
use tokio::sync::{Barrier, Mutex, OnceCell};

/// Asynchronously obtain a reference to a value that may not be ready yet.
///
/// In other words, having a `Promise<T>` is like having a `&T`, but the
/// value may depend on the result of an asynchronous computation. Calling
/// `get()` returns a Future that resolves to the inner `&T`.
#[derive(Clone)]
pub struct Promise<T> {
    item: Arc<OnceCell<T>>,
}

impl<T: Send + 'static + Sync> Promise<T> {
    /// Construct a new `Promise` where `T` is the output of the given future.
    ///
    /// The future will immediately spawn.
    pub async fn new<Fut>(fut: Fut) -> Self
    where
        Fut: Future<Output = T> + Send + 'static,
    {
        // todo: is there a way to do this without the weird af barrier stuff
        let item: Arc<OnceCell<T>> = Arc::default();

        let bar = Arc::new(Barrier::new(2));
        let bar_c = bar.clone();

        let ptr = item.clone();
        tokio::spawn(async move {
            let initer = ptr.get_or_init(|| fut);

            bar.wait().await;

            let _ = initer.await;
        });

        bar_c.wait().await;
        Self { item }
    }

    /// Get a reference to the inner value.
    pub async fn get(&self) -> &T {
        // pending is essentially a no-op future
        self.item.get_or_init(futures::future::pending).await
    }
}

/// A shared refrence to a value that may not be ready yet.
///
/// Holding a `LazyPromise<T>` is like having a `&T`, but the value may depend
/// on the result of an asynchronous computation. Calling `get()` returns a
/// Future that resolves to the inner `&T`.
///
/// `LazyPromise` is lazy in the sense that the computation starts only when
/// `get()` is first called.
#[derive(Clone)]
pub struct LazyPromise<T> {
    item: Arc<OnceCell<T>>,
    fut: Arc<Mutex<BoxFuture<'static, T>>>,
}

impl<T: Send + 'static + Sync> LazyPromise<T> {
    /// Construct a new `LazyPromise` where `T` is the output of the given
    /// future.
    ///
    /// The future will not spawn until the first time `get()` is called.
    pub fn new<Fut>(fut: Fut) -> Self
    where
        Fut: Future<Output = T> + Send + 'static,
    {
        let fut: BoxFuture<'static, T> = Box::pin(fut);

        Self {
            item: Arc::default(),
            fut: Arc::new(Mutex::new(fut)),
        }
    }

    /// Initialize the inner value.
    async fn init(&self) -> T {
        let mut t = self.fut.try_lock().expect("locked twice");
        (&mut *t).await
    }

    /// Get a reference to the inner value.
    ///
    /// The asynchronus computation will start the first time this method is
    /// called.
    pub async fn get(&self) -> &T {
        self.item.get_or_init(|| self.init()).await
    }
}

#[cfg(test)]
mod test {
    #[tokio::test]
    async fn test_promise() {
        let now = std::time::Instant::now();

        // computation starts here
        let p = super::Promise::new(async move {
            // some async computation
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            "hello".to_string()
        })
        .await;

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        p.get().await;

        assert!(now.elapsed().as_millis() > 500);
        assert!(now.elapsed().as_millis() <= 1000);
        assert_eq!(*p.get().await, "hello");
    }

    #[tokio::test]
    async fn test_lazy_promise() {
        let now = std::time::Instant::now();

        let p = super::LazyPromise::new(async move {
            // some async computation
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            "hello".to_string()
        });

        tokio::time::sleep(std::time::Duration::from_millis(250)).await;

        // computation starts here
        p.get().await;

        assert!(now.elapsed().as_millis() > 500);
        assert_eq!(*p.get().await, "hello");
    }
}
