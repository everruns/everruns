//! Built-in harness definitions.
//!
//! Each submodule defines one built-in harness (system prompt, capabilities,
//! tags, roles). The top-level `built_in_harnesses()` function collects them
//! into the ordered list consumed by `oss_built_in_harnesses()` in platform.rs.

mod base;
mod coding_container;
mod coding_daytona;
mod coding_session_sandbox;
mod data_analyst;
mod generic;
mod platform_chat;

use everruns_core::BuiltInHarnessDefinition;

/// All built-in harness definitions in provisioning order.
pub fn built_in_harnesses() -> Vec<BuiltInHarnessDefinition> {
    let internal_flags = everruns_core::InternalFeatureFlags::from_env();
    let mut harnesses = vec![
        base::definition(),
        generic::definition(),
        data_analyst::definition(),
        coding_daytona::definition(),
    ];
    if internal_flags.container_sandbox {
        harnesses.push(coding_container::definition());
    }
    harnesses.push(platform_chat::definition());
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
    fn test_coding_container_preserves_previous_provisioning_order_when_enabled() {
        let _lock = lock_env();
        let _env_guard =
            EnvVarGuard::capture(&["FEATURE_CONTAINER_SANDBOX", "FEATURE_DOCKER_CAPABILITY"]);
        unsafe { std::env::set_var("FEATURE_CONTAINER_SANDBOX", "true") };
        unsafe { std::env::remove_var("FEATURE_DOCKER_CAPABILITY") };

        let names: Vec<String> = built_in_harnesses().into_iter().map(|h| h.name).collect();

        assert_eq!(
            names,
            vec![
                "base",
                "generic",
                "data-analyst",
                "coding-daytona",
                "coding-container",
                "platform-chat",
            ]
        );
    }
}
