//! Container sandbox configuration.
//!
//! Decision: Config resolution: explicit config → env vars → defaults.
//! Decision: All resource limits have safe defaults suitable for development.

use serde::{Deserialize, Serialize};

/// Default Docker image for sandboxes.
const DEFAULT_IMAGE: &str = "ubuntu:24.04";
/// Default working directory inside containers.
const DEFAULT_WORKING_DIR: &str = "/workspace";
/// Default memory limit (2 GiB).
const DEFAULT_MEMORY_LIMIT_BYTES: i64 = 2_147_483_648;
/// Default CPU limit (1 core in nanocpus).
const DEFAULT_CPU_LIMIT_NANOCPUS: i64 = 1_000_000_000;
/// Default PID limit.
const DEFAULT_PIDS_LIMIT: i64 = 256;
/// Default auto-stop timeout (minutes).
const DEFAULT_AUTO_STOP_MINUTES: u64 = 10;

/// Configuration for the container sandbox capability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerSandboxConfig {
    /// Docker Engine host URL (env → config → "http://localhost:2375")
    #[serde(default)]
    pub docker_host: Option<String>,
    /// Container runtime (e.g., "sysbox-runc"). Empty for default runc.
    #[serde(default)]
    pub runtime: String,
    /// Container image.
    #[serde(default = "default_image")]
    pub image: String,
    /// Memory limit in bytes.
    #[serde(default = "default_memory")]
    pub memory_limit: i64,
    /// CPU limit in nanocpus.
    #[serde(default = "default_cpu")]
    pub cpu_limit: i64,
    /// PID limit.
    #[serde(default = "default_pids")]
    pub pids_limit: i64,
    /// Working directory inside the container.
    #[serde(default = "default_working_dir")]
    pub working_dir: String,
    /// Network mode (e.g., "bridge" or a custom network name).
    #[serde(default = "default_network_mode")]
    pub network_mode: String,
    /// Auto-stop timeout in minutes (for lease duration).
    #[serde(default = "default_auto_stop")]
    pub auto_stop_minutes: u64,
}

impl Default for ContainerSandboxConfig {
    fn default() -> Self {
        Self {
            docker_host: None,
            runtime: String::new(),
            image: default_image(),
            memory_limit: default_memory(),
            cpu_limit: default_cpu(),
            pids_limit: default_pids(),
            working_dir: default_working_dir(),
            network_mode: default_network_mode(),
            auto_stop_minutes: default_auto_stop(),
        }
    }
}

impl ContainerSandboxConfig {
    /// Parse config from agent capability config JSON, falling back to defaults.
    pub fn from_value(config: &serde_json::Value) -> Self {
        serde_json::from_value(config.clone()).unwrap_or_default()
    }
}

fn default_image() -> String {
    DEFAULT_IMAGE.to_string()
}
fn default_memory() -> i64 {
    DEFAULT_MEMORY_LIMIT_BYTES
}
fn default_cpu() -> i64 {
    DEFAULT_CPU_LIMIT_NANOCPUS
}
fn default_pids() -> i64 {
    DEFAULT_PIDS_LIMIT
}
fn default_working_dir() -> String {
    DEFAULT_WORKING_DIR.to_string()
}
fn default_network_mode() -> String {
    "bridge".to_string()
}
fn default_auto_stop() -> u64 {
    DEFAULT_AUTO_STOP_MINUTES
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_default_config() {
        let config = ContainerSandboxConfig::default();
        assert_eq!(config.image, "ubuntu:24.04");
        assert_eq!(config.working_dir, "/workspace");
        assert_eq!(config.memory_limit, 2_147_483_648);
        assert_eq!(config.pids_limit, 256);
        assert_eq!(config.auto_stop_minutes, 10);
    }

    #[test]
    fn test_config_from_value() {
        let config = ContainerSandboxConfig::from_value(&json!({
            "image": "node:22",
            "memory_limit": 4_294_967_296_i64,
        }));
        assert_eq!(config.image, "node:22");
        assert_eq!(config.memory_limit, 4_294_967_296);
        // Defaults for unspecified fields
        assert_eq!(config.working_dir, "/workspace");
        assert_eq!(config.pids_limit, 256);
    }

    #[test]
    fn test_config_from_invalid_value() {
        let config = ContainerSandboxConfig::from_value(&json!("not an object"));
        assert_eq!(config.image, "ubuntu:24.04"); // falls back to default
    }
}
