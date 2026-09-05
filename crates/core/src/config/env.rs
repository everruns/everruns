// Environment variable loading helpers
//
// Each function encapsulates the read → parse → fallback pattern that is
// repeated dozens of times across config structs.

use std::str::FromStr;
use std::time::Duration;

use super::ConfigError;

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

/// Read the first non-empty env var from `vars`, or return `default`.
pub fn env_string_any(vars: &[&str], default: &str) -> String {
    vars.iter()
        .find_map(|var| env_opt_string(var))
        .unwrap_or_else(|| default.to_string())
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

/// Read the first non-empty env var from `vars`.
pub fn env_opt_string_any(vars: &[&str]) -> Option<String> {
    vars.iter().find_map(|var| env_opt_string(var))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHILD: &str = "EVERRUNS_CONFIG_REVIEW_CHILD";
    const MISSING: &str = "EVERRUNS_CONFIG_REVIEW_MISSING";
    const EMPTY: &str = "EVERRUNS_CONFIG_REVIEW_EMPTY";
    const NUMBER: &str = "EVERRUNS_CONFIG_REVIEW_NUMBER";
    const BAD: &str = "EVERRUNS_CONFIG_REVIEW_BAD";
    const OVERFLOW: &str = "EVERRUNS_CONFIG_REVIEW_OVERFLOW";
    const NEGATIVE: &str = "EVERRUNS_CONFIG_REVIEW_NEGATIVE";
    const U64_OVERFLOW: &str = "EVERRUNS_CONFIG_REVIEW_U64_OVERFLOW";
    const ZERO: &str = "EVERRUNS_CONFIG_REVIEW_ZERO";
    const FIRST: &str = "EVERRUNS_CONFIG_REVIEW_FIRST";
    const SECOND: &str = "EVERRUNS_CONFIG_REVIEW_SECOND";
    const SPACE: &str = "EVERRUNS_CONFIG_REVIEW_SPACE";

    #[test]
    fn loaders_read_an_isolated_process_environment() {
        if std::env::var(CHILD).as_deref() == Ok("run") {
            assert_numeric_loading();
            assert_string_loading();
            assert_first_nonempty_precedence();
            assert_boolean_loading();
            assert_duration_units_and_fallbacks();
            assert_required_error_context();
            assert_list_order_and_invalid_items();
            println!("environment-loader assertions completed");
            return;
        }

        // Process-wide set_var/remove_var are unsafe alongside unrelated test
        // threads. Configure the child's environment before it starts instead.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                concat!(
                    module_path!(),
                    "::loaders_read_an_isolated_process_environment"
                )
                .strip_prefix("everruns_core::")
                .unwrap(),
                "--nocapture",
            ])
            .env(CHILD, "run")
            .env_remove(MISSING)
            .envs([
                (EMPTY, ""),
                (NUMBER, "42"),
                (BAD, "xyz"),
                (OVERFLOW, "4294967296"),
                (ZERO, "0"),
                (NEGATIVE, "-1"),
                (U64_OVERFLOW, "18446744073709551616"),
                (FIRST, "first"),
                (SECOND, "second"),
                (SPACE, "  "),
                ("EVERRUNS_CONFIG_REVIEW_TRUE", "true"),
                ("EVERRUNS_CONFIG_REVIEW_ONE", "1"),
                ("EVERRUNS_CONFIG_REVIEW_FALSE", "false"),
                ("EVERRUNS_CONFIG_REVIEW_UPPER", "TRUE"),
                (
                    "EVERRUNS_CONFIG_REVIEW_LIST",
                    "3, 1, bad, , -1, 4294967296, 3, 2",
                ),
            ])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "child failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // A misspelled --exact filter exits successfully with zero tests.
        assert!(
            stdout.contains("environment-loader assertions completed"),
            "child assertions did not run:\n{stdout}"
        );
    }

    fn assert_numeric_loading() {
        for (key, optional, with_default) in [
            (NUMBER, Some(42), 42),
            (ZERO, Some(0), 0),
            (MISSING, None, 17),
            (EMPTY, None, 17),
            (BAD, None, 17),
            (OVERFLOW, None, 17),
            (NEGATIVE, None, 17),
            (SPACE, None, 17),
        ] {
            assert_eq!(env_opt::<u32>(key), optional, "optional {key}");
            assert_eq!(env_or::<u32>(key, 17), with_default, "fallback {key}");
        }
    }

    fn assert_string_loading() {
        for (key, plain, optional) in [
            (FIRST, "first", Some("first")),
            (SPACE, "  ", Some("  ")),
            (EMPTY, "", None),
            (MISSING, "fallback", None),
        ] {
            assert_eq!(env_string(key, "fallback"), plain, "plain {key}");
            assert_eq!(env_opt_string(key).as_deref(), optional, "optional {key}");
        }
    }

    fn assert_first_nonempty_precedence() {
        for (keys, expected) in [
            (vec![MISSING, EMPTY, FIRST, SECOND], Some("first")),
            (vec![SECOND, FIRST], Some("second")),
            (vec![SPACE, FIRST], Some("  ")),
            (vec![MISSING, EMPTY], None),
            (vec![], None),
        ] {
            assert_eq!(env_opt_string_any(&keys).as_deref(), expected, "{keys:?}");
            assert_eq!(
                env_string_any(&keys, "fallback"),
                expected.unwrap_or("fallback"),
                "{keys:?}"
            );
        }
    }

    fn assert_boolean_loading() {
        for (key, expected) in [
            ("EVERRUNS_CONFIG_REVIEW_TRUE", true),
            ("EVERRUNS_CONFIG_REVIEW_ONE", true),
            ("EVERRUNS_CONFIG_REVIEW_FALSE", false),
            ("EVERRUNS_CONFIG_REVIEW_UPPER", false),
            (ZERO, false),
            (EMPTY, false),
            (BAD, false),
            (SPACE, false),
        ] {
            for default in [false, true] {
                assert_eq!(env_bool(key, default), expected, "{key}, default {default}");
            }
        }
        assert!(env_bool(MISSING, true));
        assert!(!env_bool(MISSING, false));
    }

    fn assert_duration_units_and_fallbacks() {
        let fallback = Duration::from_nanos(123_456_789);
        for (key, seconds, millis) in [
            (NUMBER, Duration::from_secs(42), Duration::from_millis(42)),
            (ZERO, Duration::ZERO, Duration::ZERO),
            (MISSING, fallback, fallback),
            (EMPTY, fallback, fallback),
            (BAD, fallback, fallback),
            (SPACE, fallback, fallback),
            (NEGATIVE, fallback, fallback),
            (U64_OVERFLOW, fallback, fallback),
        ] {
            assert_eq!(env_duration_secs(key, fallback), seconds, "seconds {key}");
            assert_eq!(env_duration_ms(key, fallback), millis, "millis {key}");
        }
    }

    fn assert_required_error_context() {
        assert_eq!(env_required::<u32>(NUMBER).unwrap(), 42);
        assert_eq!(env_required::<u32>(ZERO).unwrap(), 0);
        match env_required::<u32>(MISSING).unwrap_err() {
            ConfigError::Missing { var } => assert_eq!(var, MISSING),
            other => panic!("wrong missing error: {other:?}"),
        }
        for (key, original) in [(BAD, "xyz"), (EMPTY, ""), (OVERFLOW, "4294967296")] {
            match env_required::<u32>(key).unwrap_err() {
                ConfigError::Invalid { var, value, reason } => {
                    assert_eq!(var, key);
                    assert_eq!(value, original);
                    assert!(!reason.is_empty(), "invalid input needs a diagnostic");
                }
                other => panic!("wrong invalid error: {other:?}"),
            }
        }
    }

    fn assert_list_order_and_invalid_items() {
        assert_eq!(
            env_list::<u32>("EVERRUNS_CONFIG_REVIEW_LIST"),
            vec![3, 1, 3, 2]
        );
        for key in [MISSING, EMPTY, BAD, SPACE] {
            assert!(env_list::<u32>(key).is_empty(), "{key}");
        }
    }
}
