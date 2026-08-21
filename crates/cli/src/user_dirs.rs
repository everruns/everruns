//! Per-user directory lookup.
//!
//! Decision: replaces the `dirs` crate, which pulled a Redox-specific
//! passwd-parsing subtree (`dirs-sys`, `libredox`, `redox_users`, `option-ext`)
//! for two lookups. The paths below must stay byte-identical to what `dirs`
//! returned — `credentials_path()` is a stable on-disk location and moving it
//! would silently log existing users out.

use std::env;
use std::path::PathBuf;

/// The user's home directory, or `None` when the environment does not name one.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        non_empty("USERPROFILE")
    }
    #[cfg(not(windows))]
    {
        non_empty("HOME")
    }
}

/// The user's configuration directory.
///
/// Linux: `$XDG_CONFIG_HOME`, else `~/.config`.
/// macOS: `~/Library/Application Support`.
/// Windows: `%APPDATA%` (roaming).
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        home_dir().map(|home| home.join("Library").join("Application Support"))
    }
    #[cfg(windows)]
    {
        non_empty("APPDATA")
    }
    #[cfg(all(not(target_os = "macos"), not(windows)))]
    {
        xdg_dir("XDG_CONFIG_HOME", ".config")
    }
}

/// Reads an environment variable, treating an empty value as unset.
fn non_empty(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// XDG lookup: an absolute value in `key` wins, otherwise `$HOME/<fallback>`.
/// A relative XDG value is invalid per the spec and is ignored, matching `dirs`.
#[cfg(all(not(target_os = "macos"), not(windows)))]
fn xdg_dir(key: &str, fallback: &str) -> Option<PathBuf> {
    match non_empty(key) {
        Some(path) if path.is_absolute() => Some(path),
        _ => home_dir().map(|home| home.join(fallback)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_dir_reads_the_environment() {
        assert_eq!(home_dir(), env::var_os("HOME").map(PathBuf::from));
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn config_dir_falls_back_to_dot_config() {
        // XDG_CONFIG_HOME is unset in CI; when it is set, the absolute value wins.
        let expected = match env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            Some(value) if PathBuf::from(&value).is_absolute() => PathBuf::from(value),
            _ => home_dir().unwrap().join(".config"),
        };
        assert_eq!(config_dir(), Some(expected));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_uses_application_support() {
        let expected = home_dir()
            .unwrap()
            .join("Library")
            .join("Application Support");
        assert_eq!(config_dir(), Some(expected));
    }
}
