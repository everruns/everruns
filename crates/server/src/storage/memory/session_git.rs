// In-memory storage: Session Git Objects & Refs

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_core::SessionId;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Session Git Objects
    // ============================================

    pub async fn write_git_object(&self, input: CreateSessionGitObject) -> Result<()> {
        let key = (input.session_id, input.oid.clone());
        let row = SessionGitObjectRow {
            session_id: input.session_id,
            oid: input.oid,
            obj_type: input.obj_type,
            size: input.size,
            data: input.data,
        };
        // Content-addressable: insert only if absent
        self.git_objects.write().entry(key).or_insert(row);
        Ok(())
    }

    pub async fn write_git_objects_batch(
        &self,
        objects: Vec<CreateSessionGitObject>,
    ) -> Result<()> {
        let mut store = self.git_objects.write();
        for input in objects {
            let key = (input.session_id, input.oid.clone());
            let row = SessionGitObjectRow {
                session_id: input.session_id,
                oid: input.oid,
                obj_type: input.obj_type,
                size: input.size,
                data: input.data,
            };
            store.entry(key).or_insert(row);
        }
        Ok(())
    }

    pub async fn read_git_object(
        &self,
        session_id: Uuid,
        oid: &[u8],
    ) -> Result<Option<SessionGitObjectRow>> {
        let session_id = SessionId::from_uuid(session_id);
        Ok(self
            .git_objects
            .read()
            .get(&(session_id, oid.to_vec()))
            .cloned())
    }

    /// Load all git objects for a session in one call (avoids N+1).
    pub async fn load_all_git_objects(&self, session_id: Uuid) -> Result<Vec<SessionGitObjectRow>> {
        let session_id = SessionId::from_uuid(session_id);
        Ok(self
            .git_objects
            .read()
            .iter()
            .filter(|((sid, _), _)| *sid == session_id)
            .map(|(_, obj)| obj.clone())
            .collect())
    }

    pub async fn git_object_exists(&self, session_id: Uuid, oid: &[u8]) -> Result<bool> {
        let session_id = SessionId::from_uuid(session_id);
        Ok(self
            .git_objects
            .read()
            .contains_key(&(session_id, oid.to_vec())))
    }

    pub async fn read_git_object_header(
        &self,
        session_id: Uuid,
        oid: &[u8],
    ) -> Result<Option<(i16, i64)>> {
        let session_id = SessionId::from_uuid(session_id);
        Ok(self
            .git_objects
            .read()
            .get(&(session_id, oid.to_vec()))
            .map(|obj| (obj.obj_type, obj.size)))
    }

    pub async fn list_git_object_oids(&self, session_id: Uuid) -> Result<Vec<Vec<u8>>> {
        let session_id = SessionId::from_uuid(session_id);
        Ok(self
            .git_objects
            .read()
            .iter()
            .filter(|((sid, _), _)| *sid == session_id)
            .map(|((_, oid), _)| oid.clone())
            .collect())
    }

    pub async fn fork_git_objects(
        &self,
        source_session_id: Uuid,
        target_session_id: Uuid,
    ) -> Result<u64> {
        let source_id = SessionId::from_uuid(source_session_id);
        let target_id = SessionId::from_uuid(target_session_id);
        let mut store = self.git_objects.write();
        let to_copy: Vec<SessionGitObjectRow> = store
            .values()
            .filter(|obj| obj.session_id == source_id)
            .cloned()
            .collect();
        let mut count = 0u64;
        for obj in to_copy {
            let key = (target_id, obj.oid.clone());
            if let std::collections::hash_map::Entry::Vacant(e) = store.entry(key) {
                e.insert(SessionGitObjectRow {
                    session_id: target_id,
                    ..obj
                });
                count += 1;
            }
        }
        Ok(count)
    }

    // ============================================
    // Session Git Refs
    // ============================================

    pub async fn write_git_ref(&self, input: CreateSessionGitRef) -> Result<()> {
        let key = (input.session_id, input.name.clone());
        let row = SessionGitRefRow {
            session_id: input.session_id,
            name: input.name,
            target: input.target,
            is_symbolic: input.is_symbolic,
        };
        self.git_refs.write().insert(key, row);
        Ok(())
    }

    pub async fn read_git_ref(
        &self,
        session_id: Uuid,
        name: &str,
    ) -> Result<Option<SessionGitRefRow>> {
        let session_id = SessionId::from_uuid(session_id);
        Ok(self
            .git_refs
            .read()
            .get(&(session_id, name.to_string()))
            .cloned())
    }

    pub async fn delete_git_ref(&self, session_id: Uuid, name: &str) -> Result<bool> {
        let session_id = SessionId::from_uuid(session_id);
        Ok(self
            .git_refs
            .write()
            .remove(&(session_id, name.to_string()))
            .is_some())
    }

    pub async fn list_git_refs(&self, session_id: Uuid) -> Result<Vec<SessionGitRefRow>> {
        let session_id = SessionId::from_uuid(session_id);
        let mut refs: Vec<_> = self
            .git_refs
            .read()
            .values()
            .filter(|r| r.session_id == session_id)
            .cloned()
            .collect();
        refs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(refs)
    }

    pub async fn fork_git_refs(
        &self,
        source_session_id: Uuid,
        target_session_id: Uuid,
    ) -> Result<u64> {
        let source_id = SessionId::from_uuid(source_session_id);
        let target_id = SessionId::from_uuid(target_session_id);
        let mut store = self.git_refs.write();
        let to_copy: Vec<SessionGitRefRow> = store
            .values()
            .filter(|r| r.session_id == source_id)
            .cloned()
            .collect();
        let mut count = 0u64;
        for r in to_copy {
            let key = (target_id, r.name.clone());
            if let std::collections::hash_map::Entry::Vacant(e) = store.entry(key) {
                e.insert(SessionGitRefRow {
                    session_id: target_id,
                    ..r
                });
                count += 1;
            }
        }
        Ok(count)
    }
}
