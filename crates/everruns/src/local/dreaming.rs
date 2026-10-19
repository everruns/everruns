//! SQLite maintenance state, shared across processes using a local profile.
use super::db::SqliteDb;
use crate::sqlite::OptionalExtension;
use async_trait::async_trait;
use everruns_contracts::{
    error::{AgentLoopError, Result},
    typed_id::SessionId,
};
use everruns_core::dreaming::{DreamStateRow, DreamingStore};
use std::sync::Arc;
#[derive(Clone)]
pub struct LocalDreamingStore(pub SqliteDb);
impl LocalDreamingStore {
    pub fn new(db: SqliteDb) -> Result<Arc<Self>> {
        db.with_conn(|c|c.execute_batch("CREATE TABLE IF NOT EXISTS dreaming_state (org_id INTEGER NOT NULL, key TEXT NOT NULL, revision INTEGER NOT NULL, value TEXT NOT NULL, session_id TEXT, due_at INTEGER, PRIMARY KEY(org_id,key)); CREATE INDEX IF NOT EXISTS dreaming_due ON dreaming_state(due_at) WHERE due_at IS NOT NULL;" )).map_err(AgentLoopError::from)?;
        Ok(Arc::new(Self(db)))
    }
}
#[async_trait]
impl DreamingStore for LocalDreamingStore {
    async fn get(&self, org: i64, key: &str) -> Result<Option<DreamStateRow>> {
        let raw=self.0.with_conn(|c|c.query_row("SELECT revision,value,session_id,due_at FROM dreaming_state WHERE org_id=?1 AND key=?2",crate::sqlite::params![org,key],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,Option<i64>>(3)?))).optional()).map_err(AgentLoopError::from)?;
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
        let changed=self.0.with_conn(|c| match expected {Some(rev)=>c.execute("UPDATE dreaming_state SET revision=revision+1,value=?1,session_id=?2,due_at=?3 WHERE org_id=?4 AND key=?5 AND revision=?6",crate::sqlite::params![value,id.map(|s|s.to_string()),due,org,key,rev]),None=>c.execute("INSERT OR IGNORE INTO dreaming_state VALUES (?1,?2,1,?3,?4,?5)",crate::sqlite::params![org,key,value,id.map(|s|s.to_string()),due])}).map_err(AgentLoopError::from)?;
        Ok(changed == 1)
    }
    async fn due(&self, now: i64, limit: u32) -> Result<Vec<(i64, SessionId)>> {
        let rows=self.0.with_conn(|c|{ let mut q=c.prepare("SELECT org_id,session_id FROM dreaming_state WHERE due_at<=?1 AND session_id IS NOT NULL ORDER BY due_at LIMIT ?2")?;q.query_map(crate::sqlite::params![now,limit.min(100)],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?)))?.collect::<crate::sqlite::Result<Vec<_>>>()}).map_err(AgentLoopError::from)?;
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
