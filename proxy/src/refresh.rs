//! Keepalive logic for deferring resource teardown.

use futures::Future;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{sleep, Duration};

/// A `Refresher` refreshes an "owned" resource, or something that
/// that has registered takedown logic through a `RefreshHandler`.
#[derive(Clone)]
pub enum Refresher {
    One(mpsc::Sender<()>),
    Many(broadcast::Sender<()>),
}

impl Refresher {
    /// Refreshes the associated resource.
    pub fn refresh(&self) {
        match self {
            Self::One(tx) => drop(tx.try_send(())),
            Self::Many(tx) => drop(tx.send(())),
        }
    }
}

/// Manage teardown logic for some "resource" with ethereal ownership.
pub struct RefreshHandler {
    refresh: broadcast::Sender<()>,
}

impl RefreshHandler {
    /// Create a new `RefreshHandler`.
    pub fn new() -> Self {
        Self {
            refresh: broadcast::channel(1).0,
        }
    }

    /// Attach a teardown future to this handlers global refresh signal.
    ///
    /// The execution of the teardown future will begin after the given
    /// duration. Calling the `refresh` method on a `Refresher` associated
    /// with this handler will reset the timer.
    pub fn attach<F>(&self, len: u64, f: F)
    where
        F: Future + Send + 'static,
    {
        let mut many = self.refresh.subscribe();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = sleep(Duration::from_secs(len)) => break,
                    Ok(()) = many.recv() => (),
                }
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
    pub fn attach_with_local<F>(&self, len: u64, f: F) -> Refresher
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
            }

            f.await;
        });

        Refresher::One(refresh)
    }

    /// Convert this `RefreshHandler` into a `Refresher`, which
    /// can be used to refresh any takedown timers.
    pub fn into_refresher(self) -> Refresher {
        Refresher::Many(self.refresh)
    }
}
