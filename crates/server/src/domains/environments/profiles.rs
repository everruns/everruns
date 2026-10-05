//! Environment profile validation, resolution, and runtime capability mapping.

use crate::records::{
    EnvironmentContainmentLevel, EnvironmentContainmentProfile, EnvironmentDurability,
    EnvironmentEscalation, EnvironmentIdleAction, EnvironmentNetworkPolicy, EnvironmentPolicyMode,
    EnvironmentProfile, EnvironmentSelection, EnvironmentSet, EnvironmentTargetKind,
    ResolvedEnvironmentProfile,
};
use everruns_contracts::capability::CapabilityRef;
use serde_json::{Map, Value, json};

const MAX_ENVIRONMENT_PROFILES: usize = 16;
const MAX_PROVIDER_OPTIONS_BYTES: usize = 32 * 1024;
const MAX_BOOTSTRAP_COMMANDS: usize = 32;
const MAX_BOOTSTRAP_COMMAND_BYTES: usize = 8 * 1024;

const COMPUTE_CAPABILITY_IDS: &[&str] = &[
    "session_sandbox",
    "container_sandbox",
    "daytona",
    "e2b",
    "docker_container",
    "bashkit_shell",
];

/// A Session-pinned profile plus its human-facing source name.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedEnvironmentSelection {
    pub name: String,
    pub profile: ResolvedEnvironmentProfile,
}

pub fn validate_environment_set(set: &EnvironmentSet) -> Result<(), String> {
    if set.profiles.is_empty() {
        return Err("environments.profiles must contain at least one profile".to_string());
    }
    if set.profiles.len() > MAX_ENVIRONMENT_PROFILES {
        return Err(format!(
            "environments.profiles may contain at most {MAX_ENVIRONMENT_PROFILES} profiles"
        ));
    }
    if !set.profiles.contains_key(&set.default) {
        return Err(format!(
            "environments.default '{}' does not name a profile",
            set.default
        ));
    }
    if set.effective_policy() == EnvironmentPolicyMode::Fixed && set.profiles.len() != 1 {
        return Err("a fixed Environment policy must declare exactly one profile".to_string());
    }

    for (name, profile) in &set.profiles {
        crate::records::validate_addressable_name(name)
            .map_err(|error| format!("environment profile '{name}': {error}"))?;
        resolve_profile(profile)
            .map_err(|error| format!("environment profile '{name}': {error}"))?;
    }
    Ok(())
}

pub fn resolve_environment_selection(
    environments: Option<&EnvironmentSet>,
    selection: Option<&EnvironmentSelection>,
) -> Result<Option<ResolvedEnvironmentSelection>, String> {
    if let Some(set) = environments {
        validate_environment_set(set)?;
        match (set.effective_policy(), selection) {
            (EnvironmentPolicyMode::Fixed, Some(_)) => {
                return Err(
                    "this Agent has a fixed Environment; Session overrides are not allowed"
                        .to_string(),
                );
            }
            (EnvironmentPolicyMode::Selectable, Some(EnvironmentSelection::Inline(_))) => {
                return Err("this Agent only allows selecting a declared Environment".to_string());
            }
            _ => {}
        }
    } else if selection.is_some() {
        return Err("this Agent does not allow Session Environment configuration".to_string());
    }

    let (name, profile) = match selection {
        Some(EnvironmentSelection::Named { r#use }) => {
            let set = environments.ok_or_else(|| {
                "environment.use requires the selected Agent version to declare environments"
                    .to_string()
            })?;
            validate_environment_set(set)?;
            let profile = set.profiles.get(r#use).ok_or_else(|| {
                format!(
                    "environment profile '{use_name}' is not declared",
                    use_name = r#use
                )
            })?;
            (r#use.clone(), profile)
        }
        Some(EnvironmentSelection::Inline(profile)) => ("inline".to_string(), profile),
        None => {
            let Some(set) = environments else {
                return Ok(None);
            };
            validate_environment_set(set)?;
            (set.default.clone(), &set.profiles[&set.default])
        }
    };

    let profile = resolve_profile(profile)?;
    validate_target_available(&profile)?;
    Ok(Some(ResolvedEnvironmentSelection { name, profile }))
}

/// Managed Bashkit VFS used when an execution-capable Harness has no Agent
/// override. It is a real pinned Environment, not an implicit shell capability.
pub fn managed_bashkit_profile() -> EnvironmentProfile {
    EnvironmentProfile {
        source_revision_id: None,
        target: crate::records::EnvironmentTargetProfile::vfs("bashkit"),
        containment: Some(EnvironmentContainmentProfile::isolated()),
        durability: Some(EnvironmentDurability::Checkpointed),
        lifecycle: Default::default(),
        bootstrap: Default::default(),
    }
}

pub fn managed_bashkit_selection() -> ResolvedEnvironmentSelection {
    ResolvedEnvironmentSelection {
        name: "bashkit-virtual-workspace".to_string(),
        profile: resolve_profile(&managed_bashkit_profile())
            .expect("managed Bashkit Environment must remain valid"),
    }
}

pub fn selection_from_environment(
    environment: &crate::records::EnvironmentDefinition,
) -> Result<ResolvedEnvironmentSelection, String> {
    let mut authored = environment.current_revision.profile.clone();
    authored.source_revision_id = Some(environment.current_revision.public_id);
    Ok(ResolvedEnvironmentSelection {
        name: environment.name.clone(),
        profile: resolve_profile(&authored)?,
    })
}

pub fn resolve_profile(
    authored: &EnvironmentProfile,
) -> Result<ResolvedEnvironmentProfile, String> {
    validate_target_shape(authored)?;
    validate_options(&authored.target.options)?;
    validate_target_options(authored)?;
    validate_bootstrap(authored)?;

    let containment = authored
        .containment
        .clone()
        .unwrap_or_else(|| default_containment(authored.target.kind));
    let durability = authored
        .durability
        .unwrap_or_else(|| default_durability(authored.target.kind));

    validate_containment(&containment)?;
    validate_target_contract(authored.target.kind, &containment, durability)?;

    if authored.target.kind == EnvironmentTargetKind::Vfs && !authored.bootstrap.commands.is_empty()
    {
        return Err("bashkit bootstrap commands are not supported".to_string());
    }

    if authored.lifecycle.idle_after_seconds == 0
        && authored.lifecycle.idle_action != EnvironmentIdleAction::KeepRunning
    {
        return Err(
            "lifecycle.idle_after_seconds must be at least 1 unless idle_action is keep_running"
                .to_string(),
        );
    }
    if authored.lifecycle.idle_after_seconds > 86_400 {
        return Err("lifecycle.idle_after_seconds may not exceed 86400".to_string());
    }

    Ok(ResolvedEnvironmentProfile {
        source_revision_id: authored.source_revision_id,
        target: authored.target.clone(),
        containment,
        durability,
        lifecycle: authored.lifecycle.clone(),
        bootstrap: authored.bootstrap.clone(),
    })
}

fn validate_target_options(profile: &EnvironmentProfile) -> Result<(), String> {
    let options = profile
        .target
        .options
        .as_object()
        .expect("validate_options established an object");
    match profile.target.kind {
        EnvironmentTargetKind::Vfs | EnvironmentTargetKind::Host => {
            if !options.is_empty() {
                return Err(format!(
                    "{} target does not accept target.options",
                    profile.target.kind.as_str()
                ));
            }
        }
        EnvironmentTargetKind::Managed => {
            const ALLOWED: &[&str] = &[
                "snapshot",
                "size",
                "title",
                "workspace_path",
                "auto_stop_minutes",
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
                if profile.durability == Some(EnvironmentDurability::Checkpointed)
                    && !path.starts_with("/home/daytona/")
                {
                    return Err(
                        "checkpointed Daytona workspace_path must be below /home/daytona"
                            .to_string(),
                    );
                }
            }
        }
        EnvironmentTargetKind::Machine | EnvironmentTargetKind::Container => {}
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

fn validate_containment(containment: &EnvironmentContainmentProfile) -> Result<(), String> {
    if containment.escalation != EnvironmentEscalation::Never {
        return Err("containment escalation is not enforced yet; use escalation=never".to_string());
    }
    if containment.filesystem.writable_roots.len() > 32 {
        return Err(
            "containment.filesystem.writable_roots may contain at most 32 paths".to_string(),
        );
    }
    for root in &containment.filesystem.writable_roots {
        if root.len() > 4_096 || !root.starts_with('/') {
            return Err(
                "containment.filesystem.writable_roots entries must be absolute paths no longer than 4096 bytes"
                    .to_string(),
            );
        }
        if root.split('/').any(|component| component == "..") {
            return Err(
                "containment.filesystem.writable_roots entries may not contain '..'".to_string(),
            );
        }
    }
    if let EnvironmentNetworkPolicy::Allowlist { allowed_hosts } = &containment.network {
        if allowed_hosts.is_empty() || allowed_hosts.len() > 128 {
            return Err("network allowlist must contain between 1 and 128 hosts".to_string());
        }
        if allowed_hosts.iter().any(|host| {
            host.is_empty() || host.len() > 253 || host.contains('/') || host.contains('@')
        }) {
            return Err(
                "network allowlist entries must be hostnames, not URLs or credentials".to_string(),
            );
        }
    }
    Ok(())
}

/// Replace every legacy compute capability with the one selected by the pinned
/// profile. Non-compute capabilities keep their original layering and order.
pub fn apply_environment_to_capabilities(
    capabilities: &[CapabilityRef],
    environment: Option<&ResolvedEnvironmentProfile>,
) -> Vec<CapabilityRef> {
    let Some(environment) = environment else {
        return capabilities.to_vec();
    };

    let mut result = capabilities
        .iter()
        .filter(|capability| {
            !(COMPUTE_CAPABILITY_IDS.contains(&capability.id())
                || environment.target.kind == EnvironmentTargetKind::Managed
                    && capability.id() == "session_file_system")
        })
        .cloned()
        .collect::<Vec<_>>();
    result.push(capability_for_environment(environment));
    result
}

pub fn capability_for_environment(profile: &ResolvedEnvironmentProfile) -> CapabilityRef {
    match profile.target.kind {
        EnvironmentTargetKind::Vfs => {
            CapabilityRef::with_config("bashkit_shell", json!({"enable_http": false}))
        }
        EnvironmentTargetKind::Managed => {
            let mut provider_config = profile
                .target
                .options
                .as_object()
                .cloned()
                .unwrap_or_default();
            if profile.durability == EnvironmentDurability::Checkpointed {
                provider_config
                    .entry("workspace_path".to_string())
                    .or_insert_with(|| json!("/home/daytona/workspace"));
                provider_config.insert("recovery".to_string(), json!({"enabled": true}));
            }
            CapabilityRef::with_config(
                "session_sandbox",
                json!({
                    "provider": profile.target.provider,
                    "auto_start": true,
                    "idle_pause_after_seconds": profile.lifecycle.idle_after_seconds.max(1),
                    "idle_pause_enabled": profile.lifecycle.idle_action != EnvironmentIdleAction::KeepRunning,
                    "provider_config": Value::Object(provider_config),
                    "init": {"commands": profile.bootstrap.commands},
                }),
            )
        }
        EnvironmentTargetKind::Container => {
            CapabilityRef::with_config("container_sandbox", profile.target.options.clone())
        }
        // Availability validation rejects these until their control-plane
        // adapters exist, so this branch cannot reach runtime assembly.
        EnvironmentTargetKind::Host | EnvironmentTargetKind::Machine => {
            CapabilityRef::with_config("host_shell", json!({}))
        }
    }
}

fn validate_target_shape(profile: &EnvironmentProfile) -> Result<(), String> {
    let target = &profile.target;
    match target.kind {
        EnvironmentTargetKind::Host => {
            if target.provider.is_some() || target.connection_id.is_some() {
                return Err("host target accepts neither provider nor connection_id".to_string());
            }
        }
        EnvironmentTargetKind::Machine => {
            if target.connection_id.as_deref().is_none_or(str::is_empty) {
                return Err("machine target requires connection_id".to_string());
            }
            if target.provider.is_some() {
                return Err("machine target accepts connection_id, not provider".to_string());
            }
        }
        EnvironmentTargetKind::Vfs => require_provider(target.provider.as_deref(), "bashkit")?,
        EnvironmentTargetKind::Managed => require_provider(target.provider.as_deref(), "daytona")?,
        EnvironmentTargetKind::Container => require_provider(target.provider.as_deref(), "docker")?,
    }
    Ok(())
}

fn require_provider(actual: Option<&str>, expected: &str) -> Result<(), String> {
    match actual {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(format!(
            "provider '{actual}' is unsupported; expected '{expected}'"
        )),
        None => Err(format!("target requires provider '{expected}'")),
    }
}

fn validate_target_contract(
    kind: EnvironmentTargetKind,
    containment: &EnvironmentContainmentProfile,
    durability: EnvironmentDurability,
) -> Result<(), String> {
    if !containment.filesystem.writable_roots.is_empty() {
        return Err(format!(
            "{} target does not yet enforce containment.filesystem.writable_roots",
            kind.as_str()
        ));
    }
    match kind {
        EnvironmentTargetKind::Vfs => {
            require_isolated(containment, kind)?;
            if containment.network != EnvironmentNetworkPolicy::Deny {
                return Err("bashkit currently supports only network.mode=deny".to_string());
            }
            if durability != EnvironmentDurability::Checkpointed {
                return Err("bashkit durability must be checkpointed".to_string());
            }
        }
        EnvironmentTargetKind::Managed => {
            require_isolated(containment, kind)?;
            if containment.network != EnvironmentNetworkPolicy::Allow {
                return Err(
                    "daytona currently supports only network.mode=allow; an unenforced allowlist is rejected"
                        .to_string(),
                );
            }
            if durability == EnvironmentDurability::None {
                return Err(
                    "managed target durability must be checkpointed or provider_snapshot"
                        .to_string(),
                );
            }
        }
        EnvironmentTargetKind::Container => {
            require_isolated(containment, kind)?;
            if durability != EnvironmentDurability::ProviderSnapshot {
                return Err("container durability must be provider_snapshot".to_string());
            }
        }
        EnvironmentTargetKind::Host | EnvironmentTargetKind::Machine => {
            if containment.level == EnvironmentContainmentLevel::Isolated {
                return Err(format!(
                    "{} target cannot enforce isolated containment",
                    kind.as_str()
                ));
            }
            if durability != EnvironmentDurability::None {
                return Err(format!("{} target durability must be none", kind.as_str()));
            }
        }
    }
    Ok(())
}

fn require_isolated(
    containment: &EnvironmentContainmentProfile,
    kind: EnvironmentTargetKind,
) -> Result<(), String> {
    if containment.level != EnvironmentContainmentLevel::Isolated {
        return Err(format!(
            "{} target requires containment.level=isolated",
            kind.as_str()
        ));
    }
    Ok(())
}

fn default_containment(kind: EnvironmentTargetKind) -> EnvironmentContainmentProfile {
    match kind {
        EnvironmentTargetKind::Host | EnvironmentTargetKind::Machine => {
            EnvironmentContainmentProfile::uncontained()
        }
        EnvironmentTargetKind::Managed => EnvironmentContainmentProfile {
            network: EnvironmentNetworkPolicy::Allow,
            ..EnvironmentContainmentProfile::isolated()
        },
        EnvironmentTargetKind::Vfs | EnvironmentTargetKind::Container => {
            EnvironmentContainmentProfile::isolated()
        }
    }
}

fn default_durability(kind: EnvironmentTargetKind) -> EnvironmentDurability {
    match kind {
        EnvironmentTargetKind::Vfs => EnvironmentDurability::Checkpointed,
        EnvironmentTargetKind::Managed | EnvironmentTargetKind::Container => {
            EnvironmentDurability::ProviderSnapshot
        }
        EnvironmentTargetKind::Host | EnvironmentTargetKind::Machine => EnvironmentDurability::None,
    }
}

fn validate_target_available(profile: &ResolvedEnvironmentProfile) -> Result<(), String> {
    match profile.target.kind {
        EnvironmentTargetKind::Vfs => Ok(()),
        EnvironmentTargetKind::Managed
            if everruns_capabilities::create_session_sandbox_provider(
                profile.target.provider.as_deref().unwrap_or_default(),
            )
            .is_some() =>
        {
            Ok(())
        }
        EnvironmentTargetKind::Managed => Err(format!(
            "environment provider '{}' is not registered in this deployment",
            profile.target.provider.as_deref().unwrap_or("unknown")
        )),
        EnvironmentTargetKind::Host => {
            Err("host execution is disabled for this deployment".to_string())
        }
        EnvironmentTargetKind::Machine => {
            Err("registered machine execution is not implemented yet".to_string())
        }
        EnvironmentTargetKind::Container => {
            Err("container environment profiles are not enabled yet".to_string())
        }
    }
}

fn validate_options(options: &Value) -> Result<(), String> {
    let object = options
        .as_object()
        .ok_or_else(|| "target.options must be an object".to_string())?;
    let bytes = serde_json::to_vec(options).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_PROVIDER_OPTIONS_BYTES {
        return Err(format!(
            "target.options may not exceed {MAX_PROVIDER_OPTIONS_BYTES} bytes"
        ));
    }
    reject_secret_fields(object, "target.options")
}

fn reject_secret_fields(object: &Map<String, Value>, path: &str) -> Result<(), String> {
    for (key, value) in object {
        let normalized = key.to_ascii_lowercase();
        if [
            "api_key",
            "apikey",
            "token",
            "secret",
            "password",
            "credential",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
        {
            return Err(format!(
                "{path}.{key} looks like credential material; use a connection reference"
            ));
        }
        reject_secret_value(value, &format!("{path}.{key}"))?;
    }
    Ok(())
}

fn reject_secret_value(value: &Value, path: &str) -> Result<(), String> {
    match value {
        Value::Object(object) => reject_secret_fields(object, path),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                reject_secret_value(item, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn validate_bootstrap(profile: &EnvironmentProfile) -> Result<(), String> {
    if profile.bootstrap.commands.len() > MAX_BOOTSTRAP_COMMANDS {
        return Err(format!(
            "bootstrap.commands may contain at most {MAX_BOOTSTRAP_COMMANDS} commands"
        ));
    }
    if let Some((index, _)) = profile
        .bootstrap
        .commands
        .iter()
        .enumerate()
        .find(|(_, command)| command.len() > MAX_BOOTSTRAP_COMMAND_BYTES)
    {
        return Err(format!(
            "bootstrap.commands[{index}] may not exceed {MAX_BOOTSTRAP_COMMAND_BYTES} bytes"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{EnvironmentBootstrap, EnvironmentLifecycle, EnvironmentTargetProfile};
    use std::collections::BTreeMap;

    fn profile(target: crate::records::EnvironmentTargetProfile) -> EnvironmentProfile {
        EnvironmentProfile {
            source_revision_id: None,
            target,
            containment: None,
            durability: None,
            lifecycle: EnvironmentLifecycle::default(),
            bootstrap: EnvironmentBootstrap::default(),
        }
    }

    #[test]
    fn named_profile_resolves_and_replaces_legacy_compute_capabilities() {
        let set = EnvironmentSet {
            policy: None,
            default: "scratch".to_string(),
            profiles: BTreeMap::from([(
                "scratch".to_string(),
                profile(EnvironmentTargetProfile::vfs("bashkit")),
            )]),
        };
        let selected = resolve_environment_selection(Some(&set), None)
            .unwrap()
            .unwrap();
        assert_eq!(selected.name, "scratch");
        assert_eq!(
            selected.profile.durability,
            EnvironmentDurability::Checkpointed
        );

        let capabilities = vec![
            CapabilityRef::new("current_time"),
            CapabilityRef::new("daytona"),
        ];
        let mapped = apply_environment_to_capabilities(&capabilities, Some(&selected.profile));
        assert_eq!(
            mapped.iter().map(CapabilityRef::id).collect::<Vec<_>>(),
            vec!["current_time", "bashkit_shell"]
        );
    }

    #[test]
    fn fixed_policy_rejects_even_the_default_session_override() {
        let set = EnvironmentSet {
            policy: Some(EnvironmentPolicyMode::Fixed),
            default: "scratch".to_string(),
            profiles: BTreeMap::from([(
                "scratch".to_string(),
                profile(EnvironmentTargetProfile::vfs("bashkit")),
            )]),
        };
        let error = resolve_environment_selection(
            Some(&set),
            Some(&EnvironmentSelection::Named {
                r#use: "scratch".into(),
            }),
        )
        .unwrap_err();
        assert!(error.contains("fixed Environment"));
    }

    #[test]
    fn selectable_policy_rejects_inline_profiles() {
        let selected = profile(EnvironmentTargetProfile::vfs("bashkit"));
        let set = EnvironmentSet {
            policy: Some(EnvironmentPolicyMode::Selectable),
            default: "scratch".to_string(),
            profiles: BTreeMap::from([("scratch".to_string(), selected.clone())]),
        };
        let error = resolve_environment_selection(
            Some(&set),
            Some(&EnvironmentSelection::Inline(selected)),
        )
        .unwrap_err();
        assert!(error.contains("declared Environment"));
    }

    #[test]
    fn fixed_policy_requires_exactly_one_profile() {
        let selected = profile(EnvironmentTargetProfile::vfs("bashkit"));
        let set = EnvironmentSet {
            policy: Some(EnvironmentPolicyMode::Fixed),
            default: "one".to_string(),
            profiles: BTreeMap::from([
                ("one".to_string(), selected.clone()),
                ("two".to_string(), selected),
            ]),
        };

        assert!(
            validate_environment_set(&set)
                .unwrap_err()
                .contains("exactly one")
        );
    }

    #[test]
    fn managed_profile_replaces_the_session_filesystem_tool_surface() {
        let resolved =
            resolve_profile(&profile(EnvironmentTargetProfile::managed("daytona"))).unwrap();
        let capabilities = vec![
            CapabilityRef::new("session_file_system"),
            CapabilityRef::new("bashkit_shell"),
            CapabilityRef::new("current_time"),
        ];

        let mapped = apply_environment_to_capabilities(&capabilities, Some(&resolved));

        assert_eq!(
            mapped.iter().map(CapabilityRef::id).collect::<Vec<_>>(),
            vec!["current_time", "session_sandbox"]
        );
    }

    #[test]
    fn profile_rejects_embedded_credentials() {
        let mut value = profile(EnvironmentTargetProfile::managed("daytona"));
        value.target.options = json!({"api_key": "do-not-store-me"});
        assert!(
            resolve_profile(&value)
                .unwrap_err()
                .contains("connection reference")
        );
    }

    #[test]
    fn profile_rejects_provider_endpoint_overrides() {
        let mut value = profile(EnvironmentTargetProfile::managed("daytona"));
        value.target.options = json!({"api_base": "https://attacker.invalid"});
        assert!(
            resolve_profile(&value)
                .unwrap_err()
                .contains("not caller-configurable")
        );
    }

    #[test]
    fn checkpointed_daytona_profile_enables_portable_recovery() {
        let mut value = profile(EnvironmentTargetProfile::managed("daytona"));
        value.durability = Some(EnvironmentDurability::Checkpointed);
        let resolved = resolve_profile(&value).unwrap();
        let capability = capability_for_environment(&resolved);

        assert_eq!(capability.id(), "session_sandbox");
        assert_eq!(
            capability.config_value()["provider_config"]["recovery"]["enabled"],
            true
        );
        assert_eq!(
            capability.config_value()["provider_config"]["workspace_path"],
            "/home/daytona/workspace"
        );
    }

    #[test]
    fn default_must_name_a_profile() {
        let set = EnvironmentSet {
            policy: None,
            default: "missing".to_string(),
            profiles: BTreeMap::from([(
                "scratch".to_string(),
                profile(EnvironmentTargetProfile::vfs("bashkit")),
            )]),
        };
        assert!(
            validate_environment_set(&set)
                .unwrap_err()
                .contains("does not name")
        );
    }
}
