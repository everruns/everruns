//! Named durable memory directories. Stability: alpha.
use std::path::PathBuf;
/// A named directory, shared by sessions using this binding.
#[derive(Debug, Clone)]
pub struct MemoryDirectory {
    name: String,
    directory: PathBuf,
    read_only: bool,
}
impl MemoryDirectory {
    /// Mount a directory at `/memory/<name>`. The application owns isolation and sharing.
    pub fn directory(name: impl Into<String>, directory: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            directory: directory.into(),
            read_only: false,
        }
    }
    /// Refuse every mutation through agent filesystem tools.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }
}
impl crate::AgentBuilder {
    /// Attach named memory. Duplicate names and invalid path components fail at build.
    pub fn memory(mut self, memory: MemoryDirectory) -> Self {
        self.memory_directories.push((
            format!("/memory/{}", memory.name),
            memory.directory,
            memory.read_only,
        ));
        self
    }
}
