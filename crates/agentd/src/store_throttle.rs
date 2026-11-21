//! `CELL_STORE_THROTTLE_MS`: an object store that waits before every call,
//! as a throttled S3 makes its callers wait (step 10's load runs ask for it
//! on top of real S3, and locally on the file store).
//!
//! Decision: our own wrapper, not `object_store::throttle::ThrottledStore`.
//! That one panics on a local file read (`unimplemented!` for a file
//! payload), which killed executor tasks mid-turn in the first local run
//! and left their slots held for good.

use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;
use object_store::path::Path;
use object_store::{
    CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
    PutMultipartOptions, PutOptions, PutPayload, PutResult, RenameOptions, Result,
};

#[derive(Debug)]
pub struct Slow {
    inner: Arc<dyn ObjectStore>,
    wait: Duration,
}

impl Slow {
    pub fn new(inner: Arc<dyn ObjectStore>, wait: Duration) -> Self {
        Self { inner, wait }
    }
}

impl std::fmt::Display for Slow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Slow({}, {:?})", self.inner, self.wait)
    }
}

#[async_trait::async_trait]
impl ObjectStore for Slow {
    async fn put_opts(
        &self,
        location: &Path,
        payload: PutPayload,
        opts: PutOptions,
    ) -> Result<PutResult> {
        tokio::time::sleep(self.wait).await;
        self.inner.put_opts(location, payload, opts).await
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        opts: PutMultipartOptions,
    ) -> Result<Box<dyn MultipartUpload>> {
        tokio::time::sleep(self.wait).await;
        self.inner.put_multipart_opts(location, opts).await
    }

    async fn get_opts(&self, location: &Path, options: GetOptions) -> Result<GetResult> {
        tokio::time::sleep(self.wait).await;
        self.inner.get_opts(location, options).await
    }

    async fn get_ranges(&self, location: &Path, ranges: &[Range<u64>]) -> Result<Vec<Bytes>> {
        tokio::time::sleep(self.wait).await;
        self.inner.get_ranges(location, ranges).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, Result<Path>>,
    ) -> BoxStream<'static, Result<Path>> {
        let wait = self.wait;
        let locations = locations
            .then(move |location| async move {
                tokio::time::sleep(wait).await;
                location
            })
            .boxed();
        self.inner.delete_stream(locations)
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, Result<ObjectMeta>> {
        let wait = self.wait;
        let inner = self.inner.list(prefix);
        futures::stream::once(async move {
            tokio::time::sleep(wait).await;
            inner
        })
        .flatten()
        .boxed()
    }

    fn list_with_offset(
        &self,
        prefix: Option<&Path>,
        offset: &Path,
    ) -> BoxStream<'static, Result<ObjectMeta>> {
        let wait = self.wait;
        let inner = self.inner.list_with_offset(prefix, offset);
        futures::stream::once(async move {
            tokio::time::sleep(wait).await;
            inner
        })
        .flatten()
        .boxed()
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> Result<ListResult> {
        tokio::time::sleep(self.wait).await;
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(&self, from: &Path, to: &Path, options: CopyOptions) -> Result<()> {
        tokio::time::sleep(self.wait).await;
        self.inner.copy_opts(from, to, options).await
    }

    async fn rename_opts(&self, from: &Path, to: &Path, options: RenameOptions) -> Result<()> {
        tokio::time::sleep(self.wait).await;
        self.inner.rename_opts(from, to, options).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::ObjectStoreExt;

    /// The file store answers reads with a file payload, which the
    /// crate's throttle wrapper panics on; this one reads it.
    #[tokio::test]
    async fn a_slow_file_store_reads_writes_and_lists() {
        let dir = tempfile::tempdir().unwrap();
        let file = object_store::local::LocalFileSystem::new_with_prefix(dir.path()).unwrap();
        let store = Slow::new(Arc::new(file), Duration::from_millis(5));
        let path = Path::from("a/b");
        store
            .put(&path, PutPayload::from_static(b"hello"))
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let got = store.get(&path).await.unwrap().bytes().await.unwrap();
        assert_eq!(&got[..], b"hello");
        assert!(started.elapsed() >= Duration::from_millis(5));
        let listed: Vec<_> = store.list(Some(&Path::from("a"))).collect().await;
        assert_eq!(listed.len(), 1);
    }
}
