//! Bounded immutable segments over the shared blob store; disk is the local fallback.
use super::*;
use anyhow::{Result, ensure};
use futures::{Stream, StreamExt};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
pub const SEGMENT: usize = 1024 * 1024;
pub const EXTRACT_LIMIT: usize = 8 * 1024 * 1024;
pub const READ_LIMIT: usize = 1024 * 1024;
#[derive(Clone)]
pub struct Content {
    db: Arc<crate::storage::StorageBackend>,
    root: PathBuf,
}

pub struct Stored {
    pub key: String,
    pub parts: i32,
    pub bytes: i64,
    pub hash: String,
    pub extraction: Vec<u8>,
}
impl Content {
    pub fn new(db: Arc<crate::storage::StorageBackend>) -> Self {
        Self {
            db,
            root: std::env::var_os("KNOWLEDGE_STORAGE_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".local/knowledge")),
        }
    }
    pub async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<()> {
        ensure!(
            key.starts_with("knowledge/") && !key.contains(".."),
            "invalid content key"
        );
        if let Some(store) = self.db.knowledge_blob_store() {
            store
                .put(
                    key,
                    bytes,
                    &crate::storage::blob_store::BlobMetadata::new(
                        "knowledge",
                        serde_json::json!({"key":key}),
                    ),
                )
                .await
        } else {
            let target = self.root.join(key);
            tokio::fs::create_dir_all(
                target
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("content parent missing"))?,
            )
            .await?;
            let temp = target.with_extension(Uuid::new_v4().to_string());
            tokio::fs::write(&temp, bytes).await?;
            tokio::fs::rename(temp, target).await?;
            Ok(())
        }
    }
    pub async fn get(&self, key: &str) -> Result<Vec<u8>> {
        ensure!(
            key.starts_with("knowledge/") && !key.contains(".."),
            "invalid content key"
        );
        if let Some(store) = self.db.knowledge_blob_store() {
            store
                .get(key)
                .await?
                .ok_or_else(|| anyhow::anyhow!("content unavailable"))
        } else {
            Ok(tokio::fs::read(self.root.join(key)).await?)
        }
    }
    pub async fn remove(&self, key: &str, parts: i32) -> Result<()> {
        for i in 0..parts {
            let key = format!("{key}/{i}");
            if let Some(store) = self.db.knowledge_blob_store() {
                store.delete(&key).await?;
            } else {
                match tokio::fs::remove_file(self.root.join(&key)).await {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Ok(())
    }
    pub async fn store<S, E>(&self, org: i64, version: Uuid, stream: S) -> Result<Stored>
    where
        S: Stream<Item = std::result::Result<axum::body::Bytes, E>> + Send,
        E: std::fmt::Display,
    {
        futures::pin_mut!(stream);
        let key = format!("knowledge/org-{org}/versions/{version}/raw");
        let mut buffer = Vec::with_capacity(SEGMENT);
        let mut extraction = Vec::new();
        let mut digest = Sha256::new();
        let mut size = 0i64;
        let mut parts = 0;
        let result = async {
            while let Some(part) = stream.next().await {
                let part = part.map_err(|_| anyhow::anyhow!("content transfer failed"))?;
                for slice in part.chunks(SEGMENT) {
                    size = size
                        .checked_add(slice.len() as i64)
                        .ok_or_else(|| anyhow::anyhow!("file too large"))?;
                    ensure!(
                        size <= 1024i64 * 1024 * 1024 * 1024,
                        "knowledge file exceeds 1 TiB quota"
                    );
                    digest.update(slice);
                    if size <= EXTRACT_LIMIT as i64 {
                        extraction.extend_from_slice(slice);
                    } else {
                        extraction.clear();
                    }
                    let mut rest = slice;
                    while !rest.is_empty() {
                        let take = (SEGMENT - buffer.len()).min(rest.len());
                        buffer.extend_from_slice(&rest[..take]);
                        rest = &rest[take..];
                        if buffer.len() == SEGMENT {
                            self.put(&format!("{key}/{parts}"), std::mem::take(&mut buffer))
                                .await?;
                            parts += 1;
                        }
                    }
                }
            }
            if !buffer.is_empty() || parts == 0 {
                self.put(&format!("{key}/{parts}"), buffer).await?;
                parts += 1;
            }
            anyhow::Ok(())
        }
        .await;
        if let Err(error) = result {
            if self.remove(&key, parts + 1).await.is_err() {
                tracing::warn!("Knowledge staged segment cleanup failed");
            }
            return Err(error);
        }
        Ok(Stored {
            key,
            parts,
            bytes: size,
            hash: hex::encode(digest.finalize()),
            extraction,
        })
    }
    pub async fn range(&self, key: &str, size: u64, offset: u64, length: usize) -> Result<Vec<u8>> {
        ensure!(length <= EXTRACT_LIMIT, "range exceeds limit");
        let end = size.min(offset.saturating_add(length as u64));
        let mut result = Vec::with_capacity(end.saturating_sub(offset) as usize);
        if offset >= end {
            return Ok(result);
        }
        for index in offset / SEGMENT as u64..=(end - 1) / SEGMENT as u64 {
            let part = self.get(&format!("{key}/{index}")).await?;
            let start = offset.saturating_sub(index * SEGMENT as u64) as usize;
            let stop = ((end - index * SEGMENT as u64) as usize).min(part.len());
            ensure!(start <= stop, "incomplete content segment");
            result.extend_from_slice(&part[start..stop]);
        }
        Ok(result)
    }
    pub async fn text(&self, key: &str, org: i64, version: Uuid, text: &str) -> Result<String> {
        let base = format!("knowledge/org-{org}/versions/{version}/{key}");
        for (index, part) in text.as_bytes().chunks(SEGMENT).enumerate() {
            self.put(&format!("{base}/{index}"), part.to_vec()).await?;
        }
        Ok(base)
    }
}

#[cfg(test)]
mod storage_boundary_tests {
    use super::*;
    use crate::storage::blob_store::ObjectStoreBlobStore;

    #[tokio::test]
    async fn knowledge_store_survives_another_replica_without_offloading_other_files() {
        let objects = Arc::new(ObjectStoreBlobStore::in_memory());
        let db = Arc::new(
            crate::storage::StorageBackend::test_database()
                .with_knowledge_blob_store(Some(objects)),
        );
        assert!(db.blob_store().is_none());
        let first_root = tempfile::tempdir().unwrap();
        let next_root = tempfile::tempdir().unwrap();
        let mut writer = Content::new(db.clone());
        writer.root = first_root.path().into();
        let bytes = vec![0xa5; SEGMENT + 17];
        let input = bytes.clone();
        let stored = writer
            .store(
                1,
                Uuid::now_v7(),
                futures::stream::once(async {
                    Ok::<_, std::io::Error>(axum::body::Bytes::from(input))
                }),
            )
            .await
            .unwrap();
        assert_eq!(stored.parts, 2);
        let mut reader = Content::new(Arc::new(db.for_background()));
        reader.root = next_root.path().into();
        assert_eq!(
            reader
                .range(&stored.key, stored.bytes as u64, 0, bytes.len())
                .await
                .unwrap(),
            bytes
        );
        assert!(
            std::fs::read_dir(first_root.path())
                .unwrap()
                .next()
                .is_none()
        );
        reader.remove(&stored.key, stored.parts).await.unwrap();
        assert!(writer.get(&format!("{}/0", stored.key)).await.is_err());
    }
    #[tokio::test]
    async fn knowledge_override_preserves_shared_file_store_and_background_scope() {
        use crate::storage::blob_store::{BlobMetadata, BlobStore};
        let shared = Arc::new(ObjectStoreBlobStore::in_memory());
        let knowledge = Arc::new(ObjectStoreBlobStore::in_memory());
        let key = "knowledge/org-1/versions/example/raw/0";
        shared
            .put(
                key,
                b"shared".to_vec(),
                &BlobMetadata::new("test", serde_json::json!({})),
            )
            .await
            .unwrap();
        let inherited =
            crate::storage::StorageBackend::test_database().with_blob_store(Some(shared.clone()));
        assert_eq!(
            Content::new(Arc::new(inherited.clone()))
                .get(key)
                .await
                .unwrap(),
            b"shared"
        );
        let selected = inherited
            .with_knowledge_blob_store(Some(knowledge.clone()))
            .with_blob_store(Some(shared.clone()));
        let content = Content::new(Arc::new(selected.for_background()));
        content.put(key, b"knowledge".to_vec()).await.unwrap();
        assert_eq!(content.get(key).await.unwrap(), b"knowledge");
        assert_eq!(shared.get(key).await.unwrap().unwrap(), b"shared");
        assert_eq!(knowledge.get(key).await.unwrap().unwrap(), b"knowledge");
        assert!(content.put("knowledge/../private", vec![]).await.is_err());
        assert!(content.get("other/private").await.is_err());
    }
}
