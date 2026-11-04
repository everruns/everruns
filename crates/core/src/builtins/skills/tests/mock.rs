use super::*;

impl MockFileStore {
    pub(super) fn new() -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
            readonly_files: Mutex::new(std::collections::HashSet::new()),
            dirs: Mutex::new(std::collections::HashSet::new()),
            read_count: AtomicUsize::new(0),
            list_unavailable: AtomicBool::new(false),
        }
    }

    pub(super) fn read_count(&self) -> usize {
        self.read_count.load(Ordering::SeqCst)
    }

    /// Add a file and auto-create parent directories
    pub(super) fn add_file(&self, session_id: SessionId, path: &str, content: &str) {
        self.files
            .lock()
            .unwrap()
            .insert((session_id, path.to_string()), content.to_string());

        // Auto-create parent directories
        let mut dir = path.to_string();
        while let Some(idx) = dir.rfind('/') {
            if idx == 0 {
                break;
            }
            dir = dir[..idx].to_string();
            self.dirs.lock().unwrap().insert((session_id, dir.clone()));
        }
    }

    /// Add a file and mark it as readonly (simulates a capability or
    /// registry mount).
    pub(super) fn add_readonly_file(&self, session_id: SessionId, path: &str, content: &str) {
        self.add_file(session_id, path, content);
        self.readonly_files
            .lock()
            .unwrap()
            .insert((session_id, path.to_string()));
    }
}
