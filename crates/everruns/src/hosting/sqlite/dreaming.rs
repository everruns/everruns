use super::*;
use crate::sqlite::OptionalExtension;
use everruns_core::dreaming::{DreamStateRow, DreamingStore};
impl SqliteHosting {
    fn with_dream_db<T>(
        &self,
        f: impl FnOnce(&Connection) -> crate::sqlite::Result<T>,
    ) -> std::result::Result<T, AgentLoopError> {
        let db = self
            .db
            .lock()
            .map_err(|_| error("dreaming database lock poisoned"))?;
        f(&db).map_err(error)
    }
}
#[async_trait]
impl DreamingStore for SqliteHosting {
    async fn get(&self, org: i64, key: &str) -> Result<Option<DreamStateRow>> {
        let raw=self.with_dream_db(|c|c.query_row("SELECT revision,value,session_id,due_at FROM dreaming_state WHERE org_id=?1 AND key=?2",params![org,key],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<i64>>(3)?))).optional()).map_err(error)?;
        raw.map(|(revision, value, id, due_at)| {
            Ok(DreamStateRow {
                revision,
                value,
                session_id: id
                    .map(|s| s.parse())
                    .transpose()
                    .map_err(|_| AgentLoopError::store("invalid dreaming session"))?,
                due_at,
            })
        })
        .transpose()
    }
    async fn compare_exchange(
        &self,
        org: i64,
        key: &str,
        expected: Option<i64>,
        value: &str,
        id: Option<SessionId>,
        due: Option<i64>,
    ) -> Result<bool> {
        let changed=self.with_dream_db(|c| match expected {Some(rev)=>c.execute("UPDATE dreaming_state SET revision=revision+1,value=?1,session_id=?2,due_at=?3 WHERE org_id=?4 AND key=?5 AND revision=?6",params![value,id.map(|s|s.to_string()),due,org,key,rev]),None=>c.execute("INSERT OR IGNORE INTO dreaming_state VALUES (?1,?2,1,?3,?4,?5)",params![org,key,value,id.map(|s|s.to_string()),due])}).map_err(error)?;
        Ok(changed == 1)
    }
    async fn due(&self, now: i64, limit: u32) -> Result<Vec<(i64, SessionId)>> {
        let rows=self.with_dream_db(|c|{ let mut q=c.prepare("SELECT org_id,session_id FROM dreaming_state WHERE due_at<=?1 AND session_id IS NOT NULL ORDER BY due_at LIMIT ?2")?;q.query_map(params![now,limit.min(100)],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?)))?.collect::<crate::sqlite::Result<Vec<_>>>()}).map_err(error)?;
        rows.into_iter()
            .map(|(o, s)| {
                Ok((
                    o,
                    s.parse()
                        .map_err(|_| AgentLoopError::store("invalid dreaming session"))?,
                ))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn maintenance_survives_reopen_and_competing_processes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("authority.sqlite");
        let a = SqliteHosting::open(&path).unwrap();
        let b = SqliteHosting::open(&path).unwrap();
        let id = SessionId::new();
        let (x, y) = tokio::join!(
            a.compare_exchange(
                1,
                "session:test",
                None,
                "private evidence",
                Some(id),
                Some(100)
            ),
            b.compare_exchange(1, "session:test", None, "other", Some(id), Some(100))
        );
        assert_ne!(x.unwrap(), y.unwrap());
        drop(a);
        drop(b);
        let c = SqliteHosting::open(&path).unwrap();
        let saved = c.get(1, "session:test").await.unwrap().unwrap();
        assert_eq!(saved.revision, 1);
        assert!(c.get(2, "session:test").await.unwrap().is_none());
        assert!(c.due(99, 10).await.unwrap().is_empty());
        assert_eq!(c.due(100, 10).await.unwrap(), vec![(1, id)]);
        assert!(
            !c.compare_exchange(1, "session:test", Some(0), "stale", Some(id), None)
                .await
                .unwrap()
        );
    }
}
