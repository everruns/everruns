//! In-memory file attachment storage (mirrors the postgres files repository).

use super::super::models::*;
use super::InMemoryDatabase;
use crate::kernel_imports::everruns_provider::typed_id::FileId;
use anyhow::Result;
use uuid::Uuid;

impl InMemoryDatabase {
    pub async fn create_file(&self, org_id: i64, input: CreateFileRow) -> Result<FileRow> {
        let row = FileRow {
            id: FileId::new(),
            org_id,
            filename: input.filename,
            content_type: input.content_type,
            size_bytes: input.size_bytes,
            data: input.data,
            metadata: input.metadata,
            created_at: Self::now(),
        };
        self.files.write().insert(row.id, row.clone());
        Ok(row)
    }

    pub async fn get_file(&self, org_id: i64, id: Uuid) -> Result<Option<FileRow>> {
        Ok(self
            .files
            .read()
            .values()
            .find(|f| f.id.uuid() == id && f.org_id == org_id)
            .cloned())
    }

    pub async fn get_file_info(&self, org_id: i64, id: Uuid) -> Result<Option<FileInfoRow>> {
        Ok(self.get_file(org_id, id).await?.map(|f| FileInfoRow {
            id: f.id,
            org_id: f.org_id,
            filename: f.filename,
            content_type: f.content_type,
            size_bytes: f.size_bytes,
            metadata: f.metadata,
            created_at: f.created_at,
        }))
    }

    pub async fn delete_file(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut files = self.files.write();
        let key = files
            .iter()
            .find(|(_, f)| f.id.uuid() == id && f.org_id == org_id)
            .map(|(k, _)| *k);
        Ok(match key {
            Some(k) => files.remove(&k).is_some(),
            None => false,
        })
    }

    pub async fn list_files(&self, org_id: i64, limit: i64) -> Result<Vec<FileInfoRow>> {
        let mut rows: Vec<FileInfoRow> = self
            .files
            .read()
            .values()
            .filter(|f| f.org_id == org_id)
            .map(|f| FileInfoRow {
                id: f.id,
                org_id: f.org_id,
                filename: f.filename.clone(),
                content_type: f.content_type.clone(),
                size_bytes: f.size_bytes,
                metadata: f.metadata.clone(),
                created_at: f.created_at,
            })
            .collect();
        rows.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        rows.truncate(limit.max(0) as usize);
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::CreateFileRow;

    fn test_row() -> CreateFileRow {
        CreateFileRow {
            org_id: 7,
            filename: Some("report.pdf".to_string()),
            content_type: "application/pdf".to_string(),
            size_bytes: 8,
            data: b"%PDF-1.4".to_vec(),
            metadata: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn file_crud_roundtrip() {
        let db = InMemoryDatabase::default();
        let created = db.create_file(7, test_row()).await.unwrap();
        assert_eq!(created.org_id, 7);
        assert_eq!(created.filename.as_deref(), Some("report.pdf"));

        let fetched = db.get_file(7, created.id.uuid()).await.unwrap().unwrap();
        assert_eq!(fetched.data, b"%PDF-1.4");

        // Org isolation
        assert!(db.get_file(8, created.id.uuid()).await.unwrap().is_none());

        let info = db
            .get_file_info(7, created.id.uuid())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(info.size_bytes, 8);

        let list = db.list_files(7, 10).await.unwrap();
        assert_eq!(list.len(), 1);

        assert!(db.delete_file(7, created.id.uuid()).await.unwrap());
        assert!(db.get_file(7, created.id.uuid()).await.unwrap().is_none());
    }
}
