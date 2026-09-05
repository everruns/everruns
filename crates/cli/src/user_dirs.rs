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

    type EnvCase<'a> = (&'a [(&'a str, &'a str)], Option<&'a str>);

    fn check_isolated(test_name: &str, lookup: fn() -> Option<PathBuf>, cases: &[EnvCase<'_>]) {
        const PROBE: &str = "EVERRUNS_TEST_DIRECTORY_PROBE";
        const EXPECTED: &str = "EVERRUNS_TEST_DIRECTORY_EXPECTED";
        if env::var(PROBE).as_deref() == Ok(test_name) {
            let expected = env::var(EXPECTED).unwrap();
            assert_eq!(
                lookup(),
                (!expected.is_empty()).then(|| PathBuf::from(expected))
            );
            return;
        }

        // Re-enter only this test in a child with controlled environment.
        // Never mutate process-global variables while other tests run.
        for (variables, expected) in cases {
            let mut child = std::process::Command::new(env::current_exe().unwrap());
            child.args(["--exact", &format!("user_dirs::tests::{test_name}")]);
            for key in ["HOME", "USERPROFILE", "XDG_CONFIG_HOME", "APPDATA"] {
                child.env_remove(key);
            }
            let output = child
                .envs(variables.iter().copied())
                .env(PROBE, test_name)
                .env(EXPECTED, expected.unwrap_or(""))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{variables:?}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("1 passed"),
                "probe did not run: {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }

    #[test]
    fn home_dir_reads_the_environment() {
        let home_key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        check_isolated(
            "home_dir_reads_the_environment",
            home_dir,
            &[
                (&[(home_key, "/test/home")], Some("/test/home")),
                (&[(home_key, "")], None),
                (&[], None),
            ],
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn config_dir_falls_back_to_dot_config() {
        check_isolated(
            "config_dir_falls_back_to_dot_config",
            config_dir,
            &[
                (
                    &[
                        ("HOME", "/test/home"),
                        ("XDG_CONFIG_HOME", "/custom/config"),
                    ],
                    Some("/custom/config"),
                ),
                (
                    &[("HOME", "/test/home"), ("XDG_CONFIG_HOME", "relative")],
                    Some("/test/home/.config"),
                ),
                (
                    &[("HOME", "/test/home"), ("XDG_CONFIG_HOME", "")],
                    Some("/test/home/.config"),
                ),
                (&[("HOME", "/test/home")], Some("/test/home/.config")),
                (
                    &[("XDG_CONFIG_HOME", "/custom/config")],
                    Some("/custom/config"),
                ),
                (&[], None),
            ],
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn macos_uses_application_support() {
        check_isolated(
            "macos_uses_application_support",
            config_dir,
            &[
                (
                    &[("HOME", "/test/home"), ("XDG_CONFIG_HOME", "/ignored")],
                    Some("/test/home/Library/Application Support"),
                ),
                (&[("HOME", "")], None),
                (&[], None),
            ],
        );
    }

    #[test]
    #[cfg(windows)]
    fn windows_uses_roaming_appdata() {
        check_isolated(
            "windows_uses_roaming_appdata",
            config_dir,
            &[
                (
                    &[("APPDATA", "/roaming"), ("USERPROFILE", "/home")],
                    Some("/roaming"),
                ),
                (&[("APPDATA", ""), ("USERPROFILE", "/home")], None),
                (&[("USERPROFILE", "/home")], None),
            ],
        );
    }
}
