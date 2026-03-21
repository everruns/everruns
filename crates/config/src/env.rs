// Environment variable loading helpers
//
// Each function encapsulates the read → parse → fallback pattern that is
// repeated dozens of times across config structs.

use std::str::FromStr;
use std::time::Duration;

use crate::ConfigError;

/// Read an env var, parse it to `T`, or return `default`.
///
/// Silently falls back on missing or unparseable values — use [`env_required`]
/// when the caller needs to distinguish those cases.
pub fn env_or<T: FromStr>(var: &str, default: T) -> T {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Read an env var as a `String`, or return `default`.
pub fn env_string(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_string())
}

/// Read an env var as a `bool` (`"true"` or `"1"`), or return `default`.
pub fn env_bool(var: &str, default: bool) -> bool {
    std::env::var(var)
        .ok()
        .map(|v| v == "true" || v == "1")
        .unwrap_or(default)
}

/// Read an env var as a `Duration` in whole seconds, or return `default`.
pub fn env_duration_secs(var: &str, default: Duration) -> Duration {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(default)
}

/// Read an env var as a `Duration` in milliseconds, or return `default`.
pub fn env_duration_ms(var: &str, default: Duration) -> Duration {
    std::env::var(var)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(default)
}

/// Read an optional env var, parsing it to `T`. Returns `None` if the var is
/// unset or empty.
pub fn env_opt<T: FromStr>(var: &str) -> Option<T> {
    std::env::var(var)
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|v| v.parse().ok())
}

/// Read a required env var, parsing it to `T`. Returns `Err` if unset or
/// unparseable.
pub fn env_required<T: FromStr>(var: &str) -> Result<T, ConfigError> {
    let value = std::env::var(var).map_err(|_| ConfigError::Missing {
        var: var.to_string(),
    })?;
    value.parse().map_err(|_| ConfigError::Invalid {
        var: var.to_string(),
        value: value.clone(),
        reason: format!("could not parse as {}", std::any::type_name::<T>()),
    })
}

/// Read an env var as a comma-separated list of `T`. Returns an empty vec if
/// the var is unset or empty. Items that fail to parse are silently skipped.
pub fn env_list<T: FromStr>(var: &str) -> Vec<T> {
    std::env::var(var)
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.split(',')
                .filter_map(|item| item.trim().parse().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Read an env var as a `String`, or return `None` if unset/empty.
pub fn env_opt_string(var: &str) -> Option<String> {
    std::env::var(var).ok().filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    // Use a prefix to avoid collisions with real env vars in test runners
    const PREFIX: &str = "EVERRUNS_CONFIG_TEST_";

    fn set(suffix: &str, val: &str) -> String {
        let key = format!("{PREFIX}{suffix}");
        // SAFETY: Tests run single-threaded (cargo test -- --test-threads=1) or
        // use unique key prefixes so concurrent mutation is safe.
        unsafe { env::set_var(&key, val) };
        key
    }

    fn unset(suffix: &str) -> String {
        let key = format!("{PREFIX}{suffix}");
        // SAFETY: see `set` above.
        unsafe { env::remove_var(&key) };
        key
    }

    #[test]
    fn env_or_present() {
        let key = set("OR_PRESENT", "42");
        assert_eq!(env_or::<u32>(&key, 0), 42);
    }

    #[test]
    fn env_or_missing() {
        let key = unset("OR_MISSING");
        assert_eq!(env_or::<u32>(&key, 7), 7);
    }

    #[test]
    fn env_or_unparseable() {
        let key = set("OR_BAD", "not_a_number");
        assert_eq!(env_or::<u32>(&key, 99), 99);
    }

    #[test]
    fn env_string_present() {
        let key = set("STR_P", "hello");
        assert_eq!(env_string(&key, "default"), "hello");
    }

    #[test]
    fn env_string_missing() {
        let key = unset("STR_M");
        assert_eq!(env_string(&key, "fallback"), "fallback");
    }

    #[test]
    fn env_bool_true_variants() {
        let key = set("BOOL_T", "true");
        assert!(env_bool(&key, false));
        let key = set("BOOL_1", "1");
        assert!(env_bool(&key, false));
    }

    #[test]
    fn env_bool_false() {
        let key = set("BOOL_F", "false");
        assert!(!env_bool(&key, true));
    }

    #[test]
    fn env_bool_missing() {
        let key = unset("BOOL_M");
        assert!(env_bool(&key, true));
    }

    #[test]
    fn env_duration_secs_present() {
        let key = set("DUR_S", "10");
        assert_eq!(
            env_duration_secs(&key, Duration::from_secs(1)),
            Duration::from_secs(10)
        );
    }

    #[test]
    fn env_duration_secs_missing() {
        let key = unset("DUR_S_M");
        assert_eq!(
            env_duration_secs(&key, Duration::from_secs(5)),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn env_duration_ms_present() {
        let key = set("DUR_MS", "250");
        assert_eq!(
            env_duration_ms(&key, Duration::from_millis(100)),
            Duration::from_millis(250)
        );
    }

    #[test]
    fn env_opt_present() {
        let key = set("OPT_P", "42");
        assert_eq!(env_opt::<u32>(&key), Some(42));
    }

    #[test]
    fn env_opt_missing() {
        let key = unset("OPT_M");
        assert_eq!(env_opt::<u32>(&key), None::<u32>);
    }

    #[test]
    fn env_opt_empty() {
        let key = set("OPT_E", "");
        assert_eq!(env_opt::<u32>(&key), None::<u32>);
    }

    #[test]
    fn env_required_ok() {
        let key = set("REQ_OK", "100");
        assert_eq!(env_required::<u32>(&key).unwrap(), 100);
    }

    #[test]
    fn env_required_missing() {
        let key = unset("REQ_M");
        let err = env_required::<u32>(&key).unwrap_err();
        assert!(matches!(err, ConfigError::Missing { .. }));
    }

    #[test]
    fn env_required_invalid() {
        let key = set("REQ_BAD", "xyz");
        let err = env_required::<u32>(&key).unwrap_err();
        assert!(matches!(err, ConfigError::Invalid { .. }));
    }

    #[test]
    fn env_list_present() {
        let key = set("LIST_P", "1, 2, 3");
        assert_eq!(env_list::<u32>(&key), vec![1, 2, 3]);
    }

    #[test]
    fn env_list_missing() {
        let key = unset("LIST_M");
        assert_eq!(env_list::<u32>(&key), Vec::<u32>::new());
    }

    #[test]
    fn env_list_with_bad_items() {
        let key = set("LIST_B", "1,bad,3");
        assert_eq!(env_list::<u32>(&key), vec![1, 3]);
    }

    #[test]
    fn env_opt_string_present() {
        let key = set("OSTR_P", "value");
        assert_eq!(env_opt_string(&key), Some("value".to_string()));
    }

    #[test]
    fn env_opt_string_empty() {
        let key = set("OSTR_E", "");
        assert_eq!(env_opt_string(&key), None);
    }

    #[test]
    fn env_opt_string_missing() {
        let key = unset("OSTR_M");
        assert_eq!(env_opt_string(&key), None);
    }
}
