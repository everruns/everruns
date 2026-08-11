//! Built-in harness definitions.
//!
//! Decision: Only platform-essential harnesses are auto-provisioned per org —
//! `base`, `generic`, and `platform-chat`. Specialized harnesses
//! (`coding-container`, `coding-daytona`, `data-analyst`) live in the
//! `examples` module and are adopted on demand via `/v1/harness-examples`
//! and `POST /v1/harnesses/import?from-example=…`.
//!
//! Each submodule still defines one harness (system prompt, capabilities,
//! tags, roles). The top-level `built_in_harnesses()` function collects the
//! always-installed ones into the ordered list consumed by
//! `oss_built_in_harnesses()` in platform.rs.

mod base;
mod coding_container;
mod coding_daytona;
mod coding_session_sandbox;
mod data_analyst;
pub mod examples;
mod generic;
mod platform_chat;

use everruns_platform::BuiltInHarnessDefinition;

pub use examples::{
    HarnessExampleDef, LEGACY_BUILT_IN_NAMES, find_harness_example, harness_examples,
};

/// All built-in harness definitions in provisioning order.
///
/// Only platform-essential harnesses are listed here. Specialized harnesses
/// (data analyst, coding sandboxes) are adopted from `harness_examples()`.
pub fn built_in_harnesses() -> Vec<BuiltInHarnessDefinition> {
    let internal_flags = everruns_core::InternalFeatureFlags::from_env();
    let mut harnesses = vec![
        base::definition(),
        generic::definition(),
        platform_chat::definition(),
    ];
    if internal_flags.session_sandbox {
        harnesses.push(coding_session_sandbox::definition());
    }
    harnesses
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvVarGuard {
        previous: Vec<(&'static str, Option<String>)>,
    }

    impl EnvVarGuard {
        fn capture(keys: &[&'static str]) -> Self {
            Self {
                previous: keys
                    .iter()
                    .map(|&key| (key, std::env::var(key).ok()))
                    .collect(),
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            for (key, value) in self.previous.drain(..) {
                match value {
                    Some(value) => unsafe { std::env::set_var(key, value) },
                    None => unsafe { std::env::remove_var(key) },
                }
            }
        }
    }

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn built_in_list_excludes_example_harnesses() {
        let _lock = lock_env();
        let _env_guard = EnvVarGuard::capture(&[
            "FEATURE_CONTAINER_SANDBOX",
            "FEATURE_DOCKER_CAPABILITY",
            "FEATURE_SESSION_SANDBOX",
        ]);
        unsafe { std::env::set_var("FEATURE_CONTAINER_SANDBOX", "true") };
        unsafe { std::env::remove_var("FEATURE_DOCKER_CAPABILITY") };
        unsafe { std::env::remove_var("FEATURE_SESSION_SANDBOX") };

        let names: Vec<String> = built_in_harnesses().into_iter().map(|h| h.name).collect();

        // The default built-in list now contains only platform-essential
        // harnesses. Specialized coding/data harnesses moved to examples.
        assert_eq!(names, vec!["base", "generic", "platform-chat",]);
        for legacy in LEGACY_BUILT_IN_NAMES {
            assert!(
                !names.iter().any(|n| n == legacy),
                "{legacy} should no longer be a default built-in"
            );
        }
    }
}
