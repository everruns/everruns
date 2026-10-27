//! Per-target `target.options` rules for Sandbox Templates: what a caller
//! may set for each provider, checked before a template is saved.

use crate::domains::sandbox_templates::record::{
    SandboxDurability, SandboxTargetKind, SandboxTemplateSpec,
};
use everruns_contracts::session_sandbox::DESKTOP_RECORD_KEY;
use serde_json::{Map, Value};

pub(super) fn validate_target_options(profile: &SandboxTemplateSpec) -> Result<(), String> {
    let options = profile
        .target
        .options
        .as_object()
        .expect("validate_options established an object");
    match profile.target.kind {
        SandboxTargetKind::Vfs | SandboxTargetKind::Host => {
            if !options.is_empty() {
                return Err(format!(
                    "{} target does not accept target.options",
                    profile.target.kind.as_str()
                ));
            }
        }
        SandboxTargetKind::Managed if profile.target.provider.as_deref() == Some("modal") => {
            validate_modal_options(profile, options)?;
        }
        SandboxTargetKind::Managed if profile.target.provider.as_deref() == Some("e2b") => {
            validate_e2b_options(profile, options)?;
        }
        SandboxTargetKind::Managed => {
            const ALLOWED: &[&str] = &[
                "snapshot",
                "size",
                "title",
                "workspace_path",
                "auto_stop_minutes",
                "desktop",
            ];
            if let Some(key) = options.keys().find(|key| !ALLOWED.contains(&key.as_str())) {
                return Err(format!(
                    "daytona target.options.{key} is not caller-configurable"
                ));
            }
            for key in ["snapshot", "title"] {
                if let Some(value) = options.get(key)
                    && value
                        .as_str()
                        .filter(|value| !value.trim().is_empty() && value.len() <= 256)
                        .is_none()
                {
                    return Err(format!(
                        "daytona target.options.{key} must be a non-empty string no longer than 256 bytes"
                    ));
                }
            }
            if validate_desktop_option("daytona", options)?
                && let Some(key) = ["snapshot", "size"]
                    .into_iter()
                    .find(|key| options.contains_key(*key))
            {
                // The desktop ships in Daytona's default image, so a desktop
                // Sandbox cannot also pick a snapshot.
                return Err(format!(
                    "daytona target.options.{key} cannot be combined with desktop"
                ));
            }
            if let Some(size) = options.get("size")
                && !matches!(size.as_str(), Some("small" | "medium" | "large"))
            {
                return Err(
                    "daytona target.options.size must be small, medium, or large".to_string(),
                );
            }
            if let Some(minutes) = options.get("auto_stop_minutes")
                && !matches!(minutes.as_u64(), Some(1..=60))
            {
                return Err(
                    "daytona target.options.auto_stop_minutes must be between 1 and 60".to_string(),
                );
            }
            if let Some(path) = options.get("workspace_path") {
                let Some(path) = path.as_str() else {
                    return Err(
                        "daytona target.options.workspace_path must be a string".to_string()
                    );
                };
                if !is_normalized_absolute_path(path) {
                    return Err(
                        "daytona target.options.workspace_path must be a normalized absolute non-root path"
                            .to_string(),
                    );
                }
                if profile.durability == Some(SandboxDurability::Checkpointed)
                    && !path.starts_with("/home/daytona/")
                {
                    return Err(
                        "checkpointed Daytona workspace_path must be below /home/daytona"
                            .to_string(),
                    );
                }
            }
        }
        SandboxTargetKind::Machine | SandboxTargetKind::Container => {}
    }
    Ok(())
}

fn validate_e2b_options(
    profile: &SandboxTemplateSpec,
    options: &Map<String, Value>,
) -> Result<(), String> {
    const ALLOWED: &[&str] = &[
        "template",
        "title",
        "workspace_path",
        "timeout_seconds",
        "desktop",
    ];
    if profile.durability == Some(SandboxDurability::Checkpointed) {
        return Err("e2b durability must be provider_snapshot".to_string());
    }
    if let Some(key) = options.keys().find(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(format!(
            "e2b target.options.{key} is not caller-configurable"
        ));
    }
    for key in ["template", "title"] {
        if let Some(value) = options.get(key)
            && value
                .as_str()
                .filter(|value| !value.trim().is_empty() && value.len() <= 256)
                .is_none()
        {
            return Err(format!(
                "e2b target.options.{key} must be a non-empty string no longer than 256 bytes"
            ));
        }
    }
    if let Some(path) = options.get("workspace_path")
        && !path.as_str().is_some_and(is_normalized_absolute_path)
    {
        return Err(
            "e2b target.options.workspace_path must be a normalized absolute non-root path"
                .to_string(),
        );
    }
    if let Some(timeout) = options.get("timeout_seconds")
        && !matches!(timeout.as_u64(), Some(1..=86_400))
    {
        return Err("e2b target.options.timeout_seconds must be between 1 and 86400".to_string());
    }
    validate_desktop_option("e2b", options)?;
    Ok(())
}

/// `target.options.desktop`: `true`, `false`, or the computer-use settings
/// (display size, action limits, and `record: false` to skip the video) of
/// the primary Sandbox's desktop. Returns
/// whether the desktop is on.
fn validate_desktop_option(provider: &str, options: &Map<String, Value>) -> Result<bool, String> {
    match options.get("desktop") {
        None | Some(Value::Bool(false)) => Ok(false),
        Some(Value::Bool(true)) => Ok(true),
        Some(Value::Object(settings)) => {
            let mut settings = settings.clone();
            // `record` turns the desktop video off; it is not a computer-use setting.
            if let Some(record) = settings.remove(DESKTOP_RECORD_KEY)
                && !record.is_boolean()
            {
                return Err(format!(
                    "{provider} target.options.desktop.record must be true or false"
                ));
            }
            everruns_core::computer_use::ComputerUseConfig::from_value(&Value::Object(settings))
                .map_err(|error| {
                    format!("{provider} target.options.desktop is not valid: {error}")
                })?;
            Ok(true)
        }
        Some(_) => Err(format!(
            "{provider} target.options.desktop must be true, false, or an object of display settings"
        )),
    }
}

/// Modal options mirror what the provider reads; it validates them again.
fn validate_modal_options(
    profile: &SandboxTemplateSpec,
    options: &Map<String, Value>,
) -> Result<(), String> {
    const ALLOWED: &[&str] = &[
        "image",
        "runtime",
        "cpu",
        "memory_mb",
        "workspace_path",
        "title",
        "inject_connections",
    ];
    // Modal has no stop/start: pause is a provider filesystem snapshot, and
    // there is no Everruns recovery volume to checkpoint into.
    if profile.durability == Some(SandboxDurability::Checkpointed) {
        return Err("modal durability must be provider_snapshot".to_string());
    }
    if let Some(key) = options.keys().find(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(format!(
            "modal target.options.{key} is not caller-configurable"
        ));
    }
    for key in ["image", "title"] {
        if let Some(value) = options.get(key)
            && value
                .as_str()
                .filter(|value| !value.trim().is_empty() && value.len() <= 256)
                .is_none()
        {
            return Err(format!(
                "modal target.options.{key} must be a non-empty string no longer than 256 bytes"
            ));
        }
    }
    if let Some(runtime) = options.get("runtime")
        && !matches!(runtime.as_str(), Some("vm" | "gvisor"))
    {
        return Err("modal target.options.runtime must be vm or gvisor".to_string());
    }
    if let Some(cpu) = options.get("cpu")
        && !cpu
            .as_f64()
            .is_some_and(|cpu| (0.125..=64.0).contains(&cpu))
    {
        return Err("modal target.options.cpu must be between 0.125 and 64".to_string());
    }
    if let Some(memory) = options.get("memory_mb")
        && !matches!(memory.as_u64(), Some(128..=262_144))
    {
        return Err("modal target.options.memory_mb must be between 128 and 262144".to_string());
    }
    if let Some(path) = options.get("workspace_path")
        && !path.as_str().is_some_and(is_normalized_absolute_path)
    {
        return Err(
            "modal target.options.workspace_path must be a normalized absolute non-root path"
                .to_string(),
        );
    }
    Ok(())
}

fn is_normalized_absolute_path(path: &str) -> bool {
    path.starts_with('/')
        && path != "/"
        && !path.ends_with('/')
        && !path.split('/').any(|part| matches!(part, "." | ".."))
        && !path.contains("//")
}
