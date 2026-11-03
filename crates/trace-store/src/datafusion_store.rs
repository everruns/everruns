//! Read-only API bridge: DataFusion 55 uses object_store 0.13, whose cloud
//! feature pins an XML parser with known vulnerabilities. All actual I/O uses
//! the platform's patched 0.14 backend. Expose only this query's manifest index
//! keys, preserve conditional/range reads, and refuse every mutation.
use async_trait::async_trait;
use futures::{
    StreamExt, TryStreamExt,
    stream::{self, BoxStream},
};
use object_store::{ObjectStore, ObjectStoreExt};
use object_store_df as df;
use std::{collections::BTreeSet, fmt, ops::Range, sync::Arc};

#[derive(Debug)]
pub(crate) struct ReadStore {
    inner: Arc<dyn ObjectStore>,
    keys: BTreeSet<String>,
}
impl ReadStore {
    pub fn new(inner: Arc<dyn ObjectStore>, keys: BTreeSet<String>) -> Self {
        Self { inner, keys }
    }
    fn path(&self, path: &df::path::Path) -> df::Result<object_store::path::Path> {
        if !self.keys.contains(path.as_ref()) {
            return Err(df::Error::NotFound {
                path: path.to_string(),
                source: "outside selected manifests".into(),
            });
        }
        Ok(object_store::path::Path::from(path.as_ref()))
    }
}
impl fmt::Display for ReadStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "manifest-index-reader")
    }
}
fn unsupported() -> df::Error {
    df::Error::NotSupported {
        source: "manifest index bridge is read-only".into(),
    }
}
fn error(e: object_store::Error) -> df::Error {
    match e {
        object_store::Error::NotFound { path, source } => df::Error::NotFound { path, source },
        object_store::Error::Precondition { path, source } => {
            df::Error::Precondition { path, source }
        }
        object_store::Error::NotModified { path, source } => {
            df::Error::NotModified { path, source }
        }
        e => df::Error::Generic {
            store: "manifest-index-reader",
            source: Box::new(e),
        },
    }
}
fn meta(m: object_store::ObjectMeta) -> df::ObjectMeta {
    df::ObjectMeta {
        location: df::path::Path::from(m.location.as_ref()),
        last_modified: m.last_modified,
        size: m.size,
        e_tag: m.e_tag,
        version: m.version,
    }
}
#[async_trait]
impl df::ObjectStore for ReadStore {
    async fn put_opts(
        &self,
        _: &df::path::Path,
        _: df::PutPayload,
        _: df::PutOptions,
    ) -> df::Result<df::PutResult> {
        Err(unsupported())
    }
    async fn put_multipart_opts(
        &self,
        _: &df::path::Path,
        _: df::PutMultipartOptions,
    ) -> df::Result<Box<dyn df::MultipartUpload>> {
        Err(unsupported())
    }
    async fn get_opts(
        &self,
        path: &df::path::Path,
        opts: df::GetOptions,
    ) -> df::Result<df::GetResult> {
        let path = self.path(path)?;
        let range = opts.range.map(|r| match r {
            df::GetRange::Bounded(r) => object_store::GetRange::Bounded(r),
            df::GetRange::Offset(n) => object_store::GetRange::Offset(n),
            df::GetRange::Suffix(n) => object_store::GetRange::Suffix(n),
        });
        let opts = object_store::GetOptions {
            if_match: opts.if_match,
            if_none_match: opts.if_none_match,
            if_modified_since: opts.if_modified_since,
            if_unmodified_since: opts.if_unmodified_since,
            range,
            version: opts.version,
            head: opts.head,
            extensions: opts.extensions,
        };
        let result = self.inner.get_opts(&path, opts).await.map_err(error)?;
        let metadata = meta(result.meta.clone());
        let range = result.range.clone();
        Ok(df::GetResult {
            meta: metadata,
            range,
            attributes: df::Attributes::default(),
            payload: df::GetResultPayload::Stream(result.into_stream().map_err(error).boxed()),
        })
    }
    async fn get_ranges(
        &self,
        path: &df::path::Path,
        ranges: &[Range<u64>],
    ) -> df::Result<Vec<bytes::Bytes>> {
        self.inner
            .get_ranges(&self.path(path)?, ranges)
            .await
            .map_err(error)
    }
    fn delete_stream(
        &self,
        _: BoxStream<'static, df::Result<df::path::Path>>,
    ) -> BoxStream<'static, df::Result<df::path::Path>> {
        stream::once(async { Err(unsupported()) }).boxed()
    }
    fn list(
        &self,
        prefix: Option<&df::path::Path>,
    ) -> BoxStream<'static, df::Result<df::ObjectMeta>> {
        // Enumerate only known manifest keys, never the bucket. Prefix matching
        // uses path components, preserving object_store listing semantics.
        let keys: Vec<_> = self
            .keys
            .iter()
            .filter(|key| {
                prefix.is_none_or(|p| df::path::Path::from(key.as_str()).prefix_match(p).is_some())
            })
            .cloned()
            .collect();
        let inner = self.inner.clone();
        stream::iter(keys)
            .map(move |key| {
                let inner = inner.clone();
                async move {
                    inner
                        .head(&object_store::path::Path::from(key))
                        .await
                        .map(meta)
                        .map_err(error)
                }
            })
            .buffered(2)
            .boxed()
    }
    async fn list_with_delimiter(
        &self,
        prefix: Option<&df::path::Path>,
    ) -> df::Result<df::ListResult> {
        let objects: Vec<_> = self.list(prefix).try_collect().await?;
        let mut common_prefixes = BTreeSet::new();
        let mut direct = Vec::new();
        for object in objects {
            let relative = prefix.map_or_else(
                || object.location.to_string(),
                |p| {
                    object
                        .location
                        .as_ref()
                        .strip_prefix(p.as_ref())
                        .unwrap_or_default()
                        .trim_start_matches('/')
                        .to_owned()
                },
            );
            if let Some((first, _)) = relative.split_once('/') {
                common_prefixes.insert(prefix.cloned().unwrap_or_default().join(first));
            } else {
                direct.push(object);
            }
        }
        Ok(df::ListResult {
            objects: direct,
            common_prefixes: common_prefixes.into_iter().collect(),
        })
    }
    async fn copy_opts(
        &self,
        _: &df::path::Path,
        _: &df::path::Path,
        _: df::CopyOptions,
    ) -> df::Result<()> {
        Err(unsupported())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use df::{ObjectStore as _, ObjectStoreExt as _};
    #[tokio::test]
    async fn bridge_preserves_ranges_conditions_and_manifest_isolation() {
        let inner = Arc::new(object_store::memory::InMemory::new());
        inner
            .put(
                &"allowed/index.parquet".into(),
                bytes::Bytes::from_static(b"abcdefgh").into(),
            )
            .await
            .unwrap();
        inner
            .put(
                &"other/index.parquet".into(),
                bytes::Bytes::from_static(b"secret").into(),
            )
            .await
            .unwrap();
        let bridge = ReadStore::new(inner, ["allowed/index.parquet".to_owned()].into());
        let key = df::path::Path::from("allowed/index.parquet");
        assert_eq!(bridge.get_range(&key, 2..5).await.unwrap(), "cde");
        assert!(matches!(
            bridge
                .get_opts(
                    &key,
                    df::GetOptions {
                        if_match: Some("wrong".into()),
                        ..Default::default()
                    }
                )
                .await,
            Err(df::Error::Precondition { .. })
        ));
        assert!(bridge.get(&"other/index.parquet".into()).await.is_err());
        assert_eq!(
            bridge
                .list(None)
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(bridge.put(&key, bytes::Bytes::new().into()).await.is_err());
        assert!(bridge.delete(&key).await.is_err());
        assert_eq!(
            bridge.get(&key).await.unwrap().bytes().await.unwrap(),
            "abcdefgh"
        );
    }
}
