//! Tool tasks that cannot outlive Act cancellation.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// A Tokio task handle that aborts its task if the parent future is dropped
/// before the task completes. Tokio detaches a bare [`tokio::task::JoinHandle`]
/// on drop, but tool execution must not outlive Act cancellation.
pub(super) struct AbortOnDropJoinHandle<T> {
    handle: everruns_contracts::rt::JoinHandle<T>,
}

impl<T> AbortOnDropJoinHandle<T> {
    pub(super) fn new(handle: everruns_contracts::rt::JoinHandle<T>) -> Self {
        Self { handle }
    }
}

impl<T> Future for AbortOnDropJoinHandle<T> {
    type Output = std::result::Result<T, everruns_contracts::rt::JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.handle).poll(cx)
    }
}

impl<T> Drop for AbortOnDropJoinHandle<T> {
    fn drop(&mut self) {
        if !self.handle.is_finished() {
            self.handle.abort();
        }
    }
}
