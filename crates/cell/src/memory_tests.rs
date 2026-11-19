use std::collections::BTreeMap;
use std::sync::{Arc, Weak};

use everruns_contracts::dataplane::Connection;
use everruns_contracts::host_calls::{HostCallAnswer, MEMORY_FILE};
use everruns_contracts::runtime::mount_fs::MountFs;
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::InMemorySessionFileStore;
use everruns_core::session_file::{FileInfo, SessionFile};
use everruns_core::session_files::SessionFileSystem;
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::providers::OrgMaterial;

/// A control plane whose Memory is a map of paths to contents; every call
/// is recorded.
#[derive(Default)]
struct ControlPlane {
    files: Mutex<BTreeMap<String, String>>,
    asked: Mutex<Vec<Value>>,
}

fn file(session: SessionId, path: &str, content: &str) -> SessionFile {
    serde_json::from_value(json!({
        "id": uuid::Uuid::new_v4(),
        "session_id": session.uuid(),
        "path": path,
        "name": path.rsplit('/').next().unwrap_or_default(),
        "content": content,
        "encoding": "text",
        "is_directory": false,
        "is_readonly": false,
        "size_bytes": content.len(),
        "created_at": "2026-11-19T00:00:00Z",
        "updated_at": "2026-11-19T00:00:00Z",
    }))
    .unwrap()
}

fn info(session: SessionId, path: &str) -> FileInfo {
    serde_json::from_value(json!({
        "id": uuid::Uuid::new_v4(),
        "session_id": session.uuid(),
        "path": path,
        "name": path.rsplit('/').next().unwrap_or_default(),
        "is_directory": false,
        "is_readonly": false,
        "size_bytes": 1,
        "created_at": "2026-11-19T00:00:00Z",
        "updated_at": "2026-11-19T00:00:00Z",
    }))
    .unwrap()
}

#[async_trait::async_trait]
impl OrgMaterial for ControlPlane {
    fn tenant(&self, _session: SessionId) -> Option<String> {
        Some("org-a".into())
    }

    async fn connection(&self, _org: &str, _id: &str) -> anyhow::Result<Option<Connection>> {
        Ok(None)
    }

    async fn secret(&self, _org: &str, _secret_id: &str) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("no secrets")
    }

    fn looks_up(&self) -> bool {
        true
    }

    fn declared_host_calls(
        &self,
        _session: SessionId,
    ) -> Option<std::collections::BTreeSet<&'static str>> {
        Some([MEMORY_FILE].into())
    }

    async fn lookup(
        &self,
        _org: &str,
        command: &str,
        params: Value,
    ) -> anyhow::Result<HostCallAnswer> {
        assert_eq!(command, MEMORY_FILE);
        self.asked.lock().push(params.clone());
        let session = SessionId::parse(params["session_id"].as_str().unwrap()).unwrap();
        let path = params["path"].as_str().unwrap().to_owned();
        let mut files = self.files.lock();
        let ok = match params["op"].as_str().unwrap() {
            "write" => {
                let content = params["content"].as_str().unwrap().to_owned();
                files.insert(path.clone(), content.clone());
                json!(file(session, &path, &content))
            }
            "read" => match files.get(&path) {
                Some(content) => json!(file(session, &path, content)),
                None => Value::Null,
            },
            "list" => json!(
                files
                    .keys()
                    .filter(|p| p.starts_with(&format!("{path}/")))
                    .map(|p| info(session, p))
                    .collect::<Vec<_>>()
            ),
            "delete" => json!(files.remove(&path).is_some()),
            other => {
                return Ok(HostCallAnswer::Value(
                    json!({ "error": format!("no {other}") }),
                ));
            }
        };
        Ok(HostCallAnswer::Value(json!({ "ok": ok })))
    }
}

fn mounted(cp: &Arc<ControlPlane>, session: SessionId) -> MountFs {
    let material: Arc<dyn OrgMaterial> = cp.clone();
    let weak: Weak<dyn OrgMaterial> = Arc::downgrade(&material);
    // Kept alive by the test's `cp`.
    std::mem::forget(material);
    let lookups = crate::lookups::org_lookups(Some(&weak), session);
    let backends = super::attach(
        everruns_core::host::HostBackends::in_memory(),
        lookups,
        session,
        &[
            "/memory/user".into(),
            "/memory/shared".into(),
            "/etc".into(),
        ],
    );
    // A path that is no Memory mount is never mounted.
    assert_eq!(backends.memory_mounts.len(), 2);
    let mut fs = MountFs::new(Arc::new(InMemorySessionFileStore::new()));
    for (point, backend, scope) in backends.memory_mounts {
        fs = fs.with_memory_mount(point, backend, scope);
    }
    fs
}

#[tokio::test]
async fn memory_files_live_in_the_control_plane_under_their_mount() {
    let cp = Arc::new(ControlPlane::default());
    let session = SessionId::new();
    let fs = mounted(&cp, session);

    let written = fs
        .write_file(session, "/memory/user/notes.md", "likes tea", "text")
        .await
        .unwrap();
    assert_eq!(written.path, "/memory/user/notes.md");
    // The control plane got its own path, and keeps the file.
    assert_eq!(cp.asked.lock()[0]["path"], json!("/memory/user/notes.md"));
    assert_eq!(cp.asked.lock()[0]["session_id"], json!(session.to_string()));
    assert_eq!(
        cp.files
            .lock()
            .get("/memory/user/notes.md")
            .map(String::as_str),
        Some("likes tea")
    );

    // Another session's write shows at once: nothing is cached.
    cp.files
        .lock()
        .insert("/memory/shared/team.md".into(), "ship on Fridays".into());
    let read = fs
        .read_file(session, "/memory/shared/team.md")
        .await
        .unwrap()
        .expect("written by another session");
    assert_eq!(read.content.as_deref(), Some("ship on Fridays"));
    assert_eq!(read.path, "/memory/shared/team.md");

    let listed = fs.list_directory(session, "/memory/user").await.unwrap();
    let paths: Vec<_> = listed.iter().map(|info| info.path.as_str()).collect();
    assert_eq!(paths, vec!["/memory/user/notes.md"]);

    // The workspace stays the session's own: no call for it.
    let asked = cp.asked.lock().len();
    fs.write_file(session, "/workspace/draft.md", "x", "text")
        .await
        .unwrap();
    assert!(fs.read_file(session, "/memory/agent/x.md").await.is_ok());
    assert_eq!(cp.asked.lock().len(), asked);

    // The control plane's refusal is the tool's error.
    let error = fs
        .create_directory(session, "/memory/user/dir")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no mkdir"), "{error}");
}

#[tokio::test]
async fn a_memory_mount_serves_only_its_own_session() {
    let cp = Arc::new(ControlPlane::default());
    let session = SessionId::new();
    let fs = mounted(&cp, session);
    let error = fs
        .read_file(SessionId::new(), "/memory/user/notes.md")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("only its own Memory"), "{error}");
    assert!(cp.asked.lock().is_empty());
}
