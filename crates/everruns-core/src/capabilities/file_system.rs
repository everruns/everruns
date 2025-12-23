//! FileSystem Capability - for file system access (coming soon)

use super::{Capability, CapabilityId, CapabilityStatus};

/// FileSystem capability - for file system access (coming soon)
pub struct FileSystemCapability;

impl Capability for FileSystemCapability {
    fn id(&self) -> &str {
        CapabilityId::FILE_SYSTEM
    }

    fn name(&self) -> &str {
        "File System Access"
    }

    fn description(&self) -> &str {
        "Adds tools to access and manipulate files - read, write, grep, and more."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::ComingSoon
    }

    fn icon(&self) -> Option<&str> {
        Some("folder")
    }

    fn category(&self) -> Option<&str> {
        Some("File Operations")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some("You have access to file system tools. You can read, write, and search files.")
    }
}
