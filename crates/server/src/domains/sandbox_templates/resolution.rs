//! Sandbox Template validation, resolution, and runtime capability mapping.

use crate::domains::sandbox_templates::record::{
    ResolvedSandboxSpec, SandboxContainmentLevel, SandboxContainmentSpec, SandboxDurability,
    SandboxEscalation, SandboxIdleAction, SandboxNetworkPolicy, SandboxPolicy, SandboxPolicyMode,
    SandboxSelection, SandboxTargetKind, SandboxTemplateSpec,
};
use everruns_contracts::capability::CapabilityRef;
use everruns_contracts::session_sandbox::SessionSandboxCredentialSource;
use everruns_core::DeploymentGrade;
use serde_json::{Map, Value, json};

const MAX_SANDBOX_TEMPLATES: usize = 16;
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

/// Providers a `managed` target may name. Each has its own `target.options`
/// rules in `validate_target_options`.
const MANAGED_PROVIDERS: &[&str] = &["daytona", "e2b", "modal"];

/// Managed providers offered only at development grade while their integration
/// is experimental. The plugin is linked into every build, so registration
/// alone does not decide availability.
const EXPERIMENTAL_MANAGED_PROVIDERS: &[&str] = &["modal"];

/// Whether this deployment grade offers `provider` as a managed target.
pub(crate) fn managed_provider_offered(provider: &str, grade: DeploymentGrade) -> bool {
    !EXPERIMENTAL_MANAGED_PROVIDERS.contains(&provider) || grade.experimental_features_enabled()
}

/// A Session-pinned specification plus its Agent binding name.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedSandboxSelection {
    pub name: String,
    pub spec: ResolvedSandboxSpec,
}

pub fn validate_sandbox_policy(set: &SandboxPolicy) -> Result<(), String> {
    if set.templates.is_empty() {
        return Err("sandbox_policy.templates must contain at least one template".to_string());
    }
    if set.templates.len() > MAX_SANDBOX_TEMPLATES {
        return Err(format!(
            "sandbox_policy.templates may contain at most {MAX_SANDBOX_TEMPLATES} templates"
        ));
    }
    if !set.templates.contains_key(&set.default) {
        return Err(format!(
            "sandbox_policy.default '{}' does not name a template binding",
            set.default
        ));
    }
    if set.effective_mode() == SandboxPolicyMode::Fixed && set.templates.len() != 1 {
        return Err("a fixed Sandbox policy must declare exactly one template".to_string());
    }

    for (name, spec) in &set.templates {
        crate::domains::agents::record::validate_addressable_name(name)
            .map_err(|error| format!("Sandbox Template binding '{name}': {error}"))?;
        resolve_spec(spec)
            .map_err(|error| format!("Sandbox Template binding '{name}': {error}"))?;
    }
    Ok(())
}

pub fn resolve_sandbox_selection(
    sandbox_policy: Option<&SandboxPolicy>,
    selection: Option<&SandboxSelection>,
) -> Result<Option<ResolvedSandboxSelection>, String> {
    if let Some(set) = sandbox_policy {
        validate_sandbox_policy(set)?;
        match (set.effective_mode(), selection) {
            (SandboxPolicyMode::Fixed, Some(_)) => {
                return Err(
                    "this Agent has a fixed Sandbox Template; Session overrides are not allowed"
                        .to_string(),
                );
            }
            (SandboxPolicyMode::Selectable, Some(SandboxSelection::Inline(_))) => {
                return Err(
                    "this Agent only allows selecting a declared Sandbox Template".to_string(),
                );
            }
            _ => {}
        }
    } else if selection.is_some() {
        return Err("this Agent does not allow Session Sandbox configuration".to_string());
    }

    let (name, spec) = match selection {
        Some(SandboxSelection::Named { r#use }) => {
            let set = sandbox_policy.ok_or_else(|| {
                "sandbox.use requires the selected Agent version to declare a sandbox_policy"
                    .to_string()
            })?;
            validate_sandbox_policy(set)?;
            let spec = set.templates.get(r#use).ok_or_else(|| {
                format!(
                    "Sandbox Template binding '{use_name}' is not declared",
                    use_name = r#use
                )
            })?;
            (r#use.clone(), spec)
        }
        Some(SandboxSelection::Inline(profile)) => ("inline".to_string(), profile.as_ref()),
        None => {
            let Some(set) = sandbox_policy else {
                return Ok(None);
            };
            validate_sandbox_policy(set)?;
            (set.default.clone(), &set.templates[&set.default])
        }
    };

    let spec = resolve_spec(spec)?;
    validate_target_available(&spec)?;
    Ok(Some(ResolvedSandboxSelection { name, spec }))
}

/// Managed Bashkit VFS used when an execution-capable Harness has no Agent
/// override. It is a real pinned Sandbox Template, not an implicit shell capability.
pub fn managed_bashkit_sandbox_spec() -> SandboxTemplateSpec {
    SandboxTemplateSpec {
        template_revision_id: None,
        target: crate::domains::sandbox_templates::record::SandboxTargetSpec::vfs("bashkit"),
        containment: Some(SandboxContainmentSpec::isolated()),
        durability: Some(SandboxDurability::Checkpointed),
        lifecycle: Default::default(),
        bootstrap: Default::default(),
    }
}

pub fn managed_bashkit_sandbox_selection() -> ResolvedSandboxSelection {
    ResolvedSandboxSelection {
        name: "bashkit-virtual-workspace".to_string(),
        spec: resolve_spec(&managed_bashkit_sandbox_spec())
            .expect("managed Bashkit Sandbox Template must remain valid"),
    }
}

pub fn selection_from_sandbox_template(
    template: &crate::domains::sandbox_templates::record::SandboxTemplate,
) -> Result<ResolvedSandboxSelection, String> {
    let mut authored = template.current_revision.spec.clone();
    authored.template_revision_id = Some(template.current_revision.public_id);
    Ok(ResolvedSandboxSelection {
        name: template.name.clone(),
        spec: resolve_spec(&authored)?,
    })
}

pub fn resolve_spec(authored: &SandboxTemplateSpec) -> Result<ResolvedSandboxSpec, String> {
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
    validate_target_contract(
        authored.target.kind,
        authored.target.provider.as_deref(),
        &containment,
        durability,
    )?;
    if is_modal(authored) {
        modal_egress(authored, &containment)?;
    }

    if authored.target.kind == SandboxTargetKind::Vfs && !authored.bootstrap.commands.is_empty() {
        return Err("bashkit bootstrap commands are not supported".to_string());
    }

    if authored.lifecycle.idle_after_seconds == 0
        && authored.lifecycle.idle_action != SandboxIdleAction::KeepRunning
    {
        return Err(
            "lifecycle.idle_after_seconds must be at least 1 unless idle_action is keep_running"
                .to_string(),
        );
    }
    if authored.lifecycle.idle_after_seconds > 86_400 {
        return Err("lifecycle.idle_after_seconds may not exceed 86400".to_string());
    }

    let mut target = authored.target.clone();
    if target.kind == SandboxTargetKind::Managed
        && target.credential.source == SessionSandboxCredentialSource::None
    {
        // Existing managed templates predate explicit credential binding and
        // used the person starting the session. Make that legacy rule visible
        // in every resolved snapshot instead of retaining an implicit fallback.
        target.credential.source = SessionSandboxCredentialSource::SessionUser;
    }
    validate_credential_binding(&target)?;

    Ok(ResolvedSandboxSpec {
        template_revision_id: authored.template_revision_id,
        target,
        containment,
        durability,
        lifecycle: authored.lifecycle.clone(),
        bootstrap: authored.bootstrap.clone(),
    })
}

fn validate_credential_binding(
    target: &crate::domains::sandbox_templates::record::SandboxTargetSpec,
) -> Result<(), String> {
    let credential = &target.credential;
    if credential.virtual_user_id.is_some() {
        return Err(
            "credential.virtual_user_id is resolved only when the Session starts".to_string(),
        );
    }
    if target.kind != SandboxTargetKind::Managed {
        if credential != &Default::default() {
            return Err(format!(
                "{} target does not accept a provider credential",
                target.kind.as_str()
            ));
        }
        return Ok(());
    }

    match credential.source {
        SessionSandboxCredentialSource::None => {
            Err("managed target requires a credential source".to_string())
        }
        SessionSandboxCredentialSource::SessionUser | SessionSandboxCredentialSource::Agent => {
            if credential.connection_id.is_some() {
                return Err(
                    "session_user and agent credentials cannot name an organization connection"
                        .to_string(),
                );
            }
            Ok(())
        }
        SessionSandboxCredentialSource::Organization => {
            if credential.connection_id.is_none() {
                return Err(
                    "organization credential requires an explicit connection_id".to_string()
                );
            }
            Ok(())
        }
    }
}

fn validate_target_options(profile: &SandboxTemplateSpec) -> Result<(), String> {
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
    const ALLOWED: &[&str] = &["template", "title", "workspace_path", "timeout_seconds"];
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
    Ok(())
}

fn is_modal(profile: &SandboxTemplateSpec) -> bool {
    profile.target.kind == SandboxTargetKind::Managed
        && profile.target.provider.as_deref() == Some("modal")
}

/// The provider-config egress Modal enforces: the template's network policy
/// plus any connections whose tokens Modal injects. Validated with the
/// provider's own rules so a template cannot save what Modal would reject.
fn modal_egress(
    profile: &SandboxTemplateSpec,
    containment: &SandboxContainmentSpec,
) -> Result<(), String> {
    let mut egress = Map::new();
    egress.insert(
        "network".to_string(),
        modal_network_config(&containment.network),
    );
    if let Some(connections) = profile.target.options.get("inject_connections") {
        egress.insert("inject_connections".to_string(), connections.clone());
    }
    everruns_integrations::modal::egress::EgressSpec::from_json(&Value::Object(egress))
        .map(|_| ())
        .map_err(|error| format!("modal egress: {error}"))
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

fn validate_containment(containment: &SandboxContainmentSpec) -> Result<(), String> {
    if containment.escalation != SandboxEscalation::Never {
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
    if let SandboxNetworkPolicy::Allowlist { allowed_hosts } = &containment.network {
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
pub fn apply_sandbox_to_capabilities(
    capabilities: &[CapabilityRef],
    environment: Option<&ResolvedSandboxSpec>,
) -> Vec<CapabilityRef> {
    let Some(environment) = environment else {
        return capabilities.to_vec();
    };

    let mut result = capabilities
        .iter()
        .filter(|capability| {
            !(COMPUTE_CAPABILITY_IDS.contains(&capability.id())
                || environment.target.kind == SandboxTargetKind::Managed
                    && capability.id() == "session_file_system")
        })
        .cloned()
        .collect::<Vec<_>>();
    result.push(capability_for_sandbox(environment));
    result
}

pub fn capability_for_sandbox(profile: &ResolvedSandboxSpec) -> CapabilityRef {
    match profile.target.kind {
        SandboxTargetKind::Vfs => {
            CapabilityRef::with_config("bashkit_shell", json!({"enable_http": false}))
        }
        SandboxTargetKind::Managed => {
            let mut provider_config = profile
                .target
                .options
                .as_object()
                .cloned()
                .unwrap_or_default();
            if profile.target.provider.as_deref() == Some("modal") {
                // Server-written: `network` is not a caller option.
                provider_config.insert(
                    "network".to_string(),
                    modal_network_config(&profile.containment.network),
                );
            }
            if profile.durability == SandboxDurability::Checkpointed {
                provider_config
                    .entry("workspace_path".to_string())
                    .or_insert_with(|| json!("/home/daytona/workspace"));
                provider_config.insert("recovery".to_string(), json!({"enabled": true}));
            }
            CapabilityRef::with_config(
                "session_sandbox",
                json!({
                    "provider": profile.target.provider,
                    "credential": profile.target.credential,
                    "auto_start": true,
                    "idle_pause_after_seconds": profile.lifecycle.idle_after_seconds.max(1),
                    "idle_pause_enabled": profile.lifecycle.idle_action != SandboxIdleAction::KeepRunning,
                    "provider_config": Value::Object(provider_config),
                    "init": {"commands": profile.bootstrap.commands},
                }),
            )
        }
        SandboxTargetKind::Container => {
            CapabilityRef::with_config("container_sandbox", profile.target.options.clone())
        }
        // Availability validation rejects these until their control-plane
        // adapters exist, so this branch cannot reach runtime assembly.
        SandboxTargetKind::Host | SandboxTargetKind::Machine => {
            CapabilityRef::with_config("host_shell", json!({}))
        }
    }
}

fn modal_network_config(network: &SandboxNetworkPolicy) -> Value {
    match network {
        SandboxNetworkPolicy::Allow => json!({"mode": "open"}),
        SandboxNetworkPolicy::Deny => json!({"mode": "blocked"}),
        SandboxNetworkPolicy::Allowlist { allowed_hosts } => {
            json!({"mode": "allowlist", "domains": allowed_hosts})
        }
    }
}

fn validate_target_shape(profile: &SandboxTemplateSpec) -> Result<(), String> {
    let target = &profile.target;
    match target.kind {
        SandboxTargetKind::Host => {
            if target.provider.is_some() || target.connection_id.is_some() {
                return Err("host target accepts neither provider nor connection_id".to_string());
            }
        }
        SandboxTargetKind::Machine => {
            if target.connection_id.as_deref().is_none_or(str::is_empty) {
                return Err("machine target requires connection_id".to_string());
            }
            if target.provider.is_some() {
                return Err("machine target accepts connection_id, not provider".to_string());
            }
        }
        SandboxTargetKind::Vfs => require_provider(target.provider.as_deref(), "bashkit")?,
        SandboxTargetKind::Managed => match target.provider.as_deref() {
            Some(provider) if MANAGED_PROVIDERS.contains(&provider) => {}
            Some(provider) => {
                return Err(format!(
                    "provider '{provider}' is unsupported; expected one of: {}",
                    MANAGED_PROVIDERS.join(", ")
                ));
            }
            None => return Err("managed target requires a provider".to_string()),
        },
        SandboxTargetKind::Container => require_provider(target.provider.as_deref(), "docker")?,
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
    kind: SandboxTargetKind,
    provider: Option<&str>,
    containment: &SandboxContainmentSpec,
    durability: SandboxDurability,
) -> Result<(), String> {
    if !containment.filesystem.writable_roots.is_empty() {
        return Err(format!(
            "{} target does not yet enforce containment.filesystem.writable_roots",
            kind.as_str()
        ));
    }
    match kind {
        SandboxTargetKind::Vfs => {
            require_isolated(containment, kind)?;
            if containment.network != SandboxNetworkPolicy::Deny {
                return Err("bashkit currently supports only network.mode=deny".to_string());
            }
            if durability != SandboxDurability::Checkpointed {
                return Err("bashkit durability must be checkpointed".to_string());
            }
        }
        SandboxTargetKind::Managed => {
            require_isolated(containment, kind)?;
            // Modal enforces deny and allowlists itself (`modal_egress`).
            if provider != Some("modal") && containment.network != SandboxNetworkPolicy::Allow {
                return Err(
                    "managed targets currently support only network.mode=allow; an unenforced allowlist is rejected"
                        .to_string(),
                );
            }
            if durability == SandboxDurability::None {
                return Err(
                    "managed target durability must be checkpointed or provider_snapshot"
                        .to_string(),
                );
            }
        }
        SandboxTargetKind::Container => {
            require_isolated(containment, kind)?;
            if durability != SandboxDurability::ProviderSnapshot {
                return Err("container durability must be provider_snapshot".to_string());
            }
        }
        SandboxTargetKind::Host | SandboxTargetKind::Machine => {
            if containment.level == SandboxContainmentLevel::Isolated {
                return Err(format!(
                    "{} target cannot enforce isolated containment",
                    kind.as_str()
                ));
            }
            if durability != SandboxDurability::None {
                return Err(format!("{} target durability must be none", kind.as_str()));
            }
        }
    }
    Ok(())
}

fn require_isolated(
    containment: &SandboxContainmentSpec,
    kind: SandboxTargetKind,
) -> Result<(), String> {
    if containment.level != SandboxContainmentLevel::Isolated {
        return Err(format!(
            "{} target requires containment.level=isolated",
            kind.as_str()
        ));
    }
    Ok(())
}

fn default_containment(kind: SandboxTargetKind) -> SandboxContainmentSpec {
    match kind {
        SandboxTargetKind::Host | SandboxTargetKind::Machine => {
            SandboxContainmentSpec::uncontained()
        }
        SandboxTargetKind::Managed => SandboxContainmentSpec {
            network: SandboxNetworkPolicy::Allow,
            ..SandboxContainmentSpec::isolated()
        },
        SandboxTargetKind::Vfs | SandboxTargetKind::Container => SandboxContainmentSpec::isolated(),
    }
}

fn default_durability(kind: SandboxTargetKind) -> SandboxDurability {
    match kind {
        SandboxTargetKind::Vfs => SandboxDurability::Checkpointed,
        SandboxTargetKind::Managed | SandboxTargetKind::Container => {
            SandboxDurability::ProviderSnapshot
        }
        SandboxTargetKind::Host | SandboxTargetKind::Machine => SandboxDurability::None,
    }
}

fn validate_target_available(profile: &ResolvedSandboxSpec) -> Result<(), String> {
    validate_target_available_for_grade(profile, DeploymentGrade::from_env())
}

fn validate_target_available_for_grade(
    profile: &ResolvedSandboxSpec,
    grade: DeploymentGrade,
) -> Result<(), String> {
    match profile.target.kind {
        SandboxTargetKind::Vfs => Ok(()),
        SandboxTargetKind::Managed
            if !managed_provider_offered(
                profile.target.provider.as_deref().unwrap_or_default(),
                grade,
            ) =>
        {
            Err(format!(
                "environment provider '{}' is experimental and offered only at development grade",
                profile.target.provider.as_deref().unwrap_or("unknown")
            ))
        }
        SandboxTargetKind::Managed
            if everruns_capabilities::create_session_sandbox_provider(
                profile.target.provider.as_deref().unwrap_or_default(),
            )
            .is_some() =>
        {
            Ok(())
        }
        SandboxTargetKind::Managed => Err(format!(
            "environment provider '{}' is not registered in this deployment",
            profile.target.provider.as_deref().unwrap_or("unknown")
        )),
        SandboxTargetKind::Host => {
            Err("host execution is disabled for this deployment".to_string())
        }
        SandboxTargetKind::Machine => {
            Err("registered machine execution is not implemented yet".to_string())
        }
        SandboxTargetKind::Container => {
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

fn validate_bootstrap(profile: &SandboxTemplateSpec) -> Result<(), String> {
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
    use crate::domains::sandbox_templates::record::{
        SandboxBootstrap, SandboxLifecycle, SandboxTargetSpec,
    };
    use std::collections::BTreeMap;

    fn profile(
        target: crate::domains::sandbox_templates::record::SandboxTargetSpec,
    ) -> SandboxTemplateSpec {
        SandboxTemplateSpec {
            template_revision_id: None,
            target,
            containment: None,
            durability: None,
            lifecycle: SandboxLifecycle::default(),
            bootstrap: SandboxBootstrap::default(),
        }
    }

    #[test]
    fn named_profile_resolves_and_replaces_legacy_compute_capabilities() {
        let set = SandboxPolicy {
            mode: None,
            default: "scratch".to_string(),
            templates: BTreeMap::from([(
                "scratch".to_string(),
                profile(SandboxTargetSpec::vfs("bashkit")),
            )]),
        };
        let selected = resolve_sandbox_selection(Some(&set), None)
            .unwrap()
            .unwrap();
        assert_eq!(selected.name, "scratch");
        assert_eq!(selected.spec.durability, SandboxDurability::Checkpointed);

        let capabilities = vec![
            CapabilityRef::new("current_time"),
            CapabilityRef::new("daytona"),
        ];
        let mapped = apply_sandbox_to_capabilities(&capabilities, Some(&selected.spec));
        assert_eq!(
            mapped.iter().map(CapabilityRef::id).collect::<Vec<_>>(),
            vec!["current_time", "bashkit_shell"]
        );
    }

    #[test]
    fn fixed_policy_rejects_even_the_default_session_override() {
        let set = SandboxPolicy {
            mode: Some(SandboxPolicyMode::Fixed),
            default: "scratch".to_string(),
            templates: BTreeMap::from([(
                "scratch".to_string(),
                profile(SandboxTargetSpec::vfs("bashkit")),
            )]),
        };
        let error = resolve_sandbox_selection(
            Some(&set),
            Some(&SandboxSelection::Named {
                r#use: "scratch".into(),
            }),
        )
        .unwrap_err();
        assert!(error.contains("fixed Sandbox Template"));
    }

    #[test]
    fn selectable_policy_rejects_inline_profiles() {
        let selected = profile(SandboxTargetSpec::vfs("bashkit"));
        let set = SandboxPolicy {
            mode: Some(SandboxPolicyMode::Selectable),
            default: "scratch".to_string(),
            templates: BTreeMap::from([("scratch".to_string(), selected.clone())]),
        };
        let error = resolve_sandbox_selection(
            Some(&set),
            Some(&SandboxSelection::Inline(Box::new(selected))),
        )
        .unwrap_err();
        assert!(error.contains("declared Sandbox Template"));
    }

    #[test]
    fn fixed_policy_requires_exactly_one_profile() {
        let selected = profile(SandboxTargetSpec::vfs("bashkit"));
        let set = SandboxPolicy {
            mode: Some(SandboxPolicyMode::Fixed),
            default: "one".to_string(),
            templates: BTreeMap::from([
                ("one".to_string(), selected.clone()),
                ("two".to_string(), selected),
            ]),
        };

        assert!(
            validate_sandbox_policy(&set)
                .unwrap_err()
                .contains("exactly one")
        );
    }

    #[test]
    fn managed_profile_replaces_the_session_filesystem_tool_surface() {
        let resolved = resolve_spec(&profile(SandboxTargetSpec::managed("daytona"))).unwrap();
        assert_eq!(
            resolved.target.credential.source,
            SessionSandboxCredentialSource::SessionUser
        );
        let capabilities = vec![
            CapabilityRef::new("session_file_system"),
            CapabilityRef::new("bashkit_shell"),
            CapabilityRef::new("current_time"),
        ];

        let mapped = apply_sandbox_to_capabilities(&capabilities, Some(&resolved));

        assert_eq!(
            mapped.iter().map(CapabilityRef::id).collect::<Vec<_>>(),
            vec!["current_time", "session_sandbox"]
        );
    }

    #[test]
    fn organization_credential_requires_an_exact_connection() {
        let mut value = profile(SandboxTargetSpec::managed("daytona"));
        value.target.credential.source = SessionSandboxCredentialSource::Organization;
        assert!(resolve_spec(&value).unwrap_err().contains("connection_id"));

        value.target.credential.connection_id = Some(uuid::Uuid::new_v4());
        let resolved = resolve_spec(&value).unwrap();
        assert_eq!(
            capability_for_sandbox(&resolved).config_value()["credential"]["source"],
            "organization"
        );
    }

    #[test]
    fn non_managed_target_rejects_provider_credentials() {
        let mut value = profile(SandboxTargetSpec::vfs("bashkit"));
        value.target.credential.source = SessionSandboxCredentialSource::Agent;
        assert!(
            resolve_spec(&value)
                .unwrap_err()
                .contains("does not accept")
        );
    }

    #[test]
    fn profile_rejects_embedded_credentials() {
        let mut value = profile(SandboxTargetSpec::managed("daytona"));
        value.target.options = json!({"api_key": "do-not-store-me"});
        assert!(
            resolve_spec(&value)
                .unwrap_err()
                .contains("connection reference")
        );
    }

    #[test]
    fn profile_rejects_provider_endpoint_overrides() {
        let mut value = profile(SandboxTargetSpec::managed("daytona"));
        value.target.options = json!({"api_base": "https://attacker.invalid"});
        assert!(
            resolve_spec(&value)
                .unwrap_err()
                .contains("not caller-configurable")
        );
    }

    #[test]
    fn checkpointed_daytona_profile_enables_portable_recovery() {
        let mut value = profile(SandboxTargetSpec::managed("daytona"));
        value.durability = Some(SandboxDurability::Checkpointed);
        let resolved = resolve_spec(&value).unwrap();
        let capability = capability_for_sandbox(&resolved);

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
    fn modal_profile_maps_to_a_session_sandbox_with_its_options() {
        let mut value = profile(SandboxTargetSpec::managed("modal"));
        value.target.options = json!({
            "runtime": "gvisor", "image": "node:22", "cpu": 0.5, "memory_mb": 1024,
            "workspace_path": "/srv/app", "title": "web"
        });
        let resolved = resolve_spec(&value).unwrap();
        assert_eq!(resolved.durability, SandboxDurability::ProviderSnapshot);
        let capability = capability_for_sandbox(&resolved);
        let config = capability.config_value();
        assert_eq!(capability.id(), "session_sandbox");
        assert_eq!(config["provider"], "modal");
        assert_eq!(config["provider_config"]["runtime"], "gvisor");
        assert_eq!(config["provider_config"]["workspace_path"], "/srv/app");
        assert!(config["provider_config"].get("recovery").is_none());
    }

    #[test]
    fn modal_profile_rejects_what_the_provider_cannot_do() {
        let cases = [
            (json!({"snapshot": "x"}), "not caller-configurable"),
            (
                json!({"_test_server_url": "http://x"}),
                "not caller-configurable",
            ),
            (json!({"runtime": "firecracker"}), "vm or gvisor"),
            (json!({"cpu": 0}), "cpu"),
            (json!({"memory_mb": 64}), "memory_mb"),
            (json!({"workspace_path": "/a/../b"}), "workspace_path"),
            (json!({"image": ""}), "image"),
        ];
        for (options, expected) in cases {
            let mut value = profile(SandboxTargetSpec::managed("modal"));
            value.target.options = options.clone();
            let error = resolve_spec(&value).unwrap_err();
            assert!(error.contains(expected), "{options}: {error}");
        }

        let mut checkpointed = profile(SandboxTargetSpec::managed("modal"));
        checkpointed.durability = Some(SandboxDurability::Checkpointed);
        assert!(
            resolve_spec(&checkpointed)
                .unwrap_err()
                .contains("provider_snapshot")
        );

        let unknown = profile(SandboxTargetSpec::managed("fly"));
        assert!(
            resolve_spec(&unknown)
                .unwrap_err()
                .contains("daytona, e2b, modal")
        );
    }

    #[test]
    fn e2b_profile_maps_provider_options_and_rejects_checkpointed_durability() {
        let mut value = profile(SandboxTargetSpec::managed("e2b"));
        value.target.options = json!({
            "template": "base",
            "timeout_seconds": 3600,
            "workspace_path": "/home/user/workspace"
        });
        let resolved = resolve_spec(&value).unwrap();
        let config = capability_for_sandbox(&resolved).config_value().clone();
        assert_eq!(config["provider"], "e2b");
        assert_eq!(config["provider_config"]["template"], "base");
        assert_eq!(config["provider_config"]["timeout_seconds"], 3600);

        value.durability = Some(SandboxDurability::Checkpointed);
        assert!(
            resolve_spec(&value)
                .unwrap_err()
                .contains("provider_snapshot")
        );
    }

    #[test]
    fn modal_enforces_the_template_network_policy() {
        let modal = |network: SandboxNetworkPolicy, options: Value| {
            let mut value = profile(SandboxTargetSpec::managed("modal"));
            value.containment = Some(SandboxContainmentSpec {
                network,
                ..SandboxContainmentSpec::isolated()
            });
            value.target.options = options;
            resolve_spec(&value)
        };
        let config = |resolved: &ResolvedSandboxSpec| {
            capability_for_sandbox(resolved).config_value()["provider_config"].clone()
        };

        let denied = modal(SandboxNetworkPolicy::Deny, json!({})).unwrap();
        assert_eq!(config(&denied)["network"], json!({"mode": "blocked"}));
        let listed = modal(
            SandboxNetworkPolicy::Allowlist {
                allowed_hosts: vec!["pypi.org".into(), "*.pythonhosted.org".into()],
            },
            json!({}),
        )
        .unwrap();
        assert_eq!(
            config(&listed)["network"],
            json!({"mode": "allowlist", "domains": ["pypi.org", "*.pythonhosted.org"]})
        );
        let injected = modal(
            SandboxNetworkPolicy::Allow,
            json!({"inject_connections": ["github"]}),
        )
        .unwrap();
        assert_eq!(config(&injected)["network"], json!({"mode": "open"}));
        assert_eq!(config(&injected)["inject_connections"], json!(["github"]));

        for (network, options, expected) in [
            (
                SandboxNetworkPolicy::Deny,
                json!({"inject_connections": ["github"]}),
                "needs network",
            ),
            (
                SandboxNetworkPolicy::Allowlist {
                    allowed_hosts: vec!["github.com".into()],
                },
                json!({"inject_connections": ["github"]}),
                "limited to domains",
            ),
            (
                SandboxNetworkPolicy::Allow,
                json!({"inject_connections": ["slack"]}),
                "cannot be injected",
            ),
            (
                SandboxNetworkPolicy::Allowlist {
                    allowed_hosts: vec!["localhost".into()],
                },
                json!({}),
                "not a domain",
            ),
        ] {
            let error = modal(network, options).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }

        // Daytona still cannot enforce anything but open egress.
        let mut daytona = profile(SandboxTargetSpec::managed("daytona"));
        daytona.containment = Some(SandboxContainmentSpec {
            network: SandboxNetworkPolicy::Deny,
            ..SandboxContainmentSpec::isolated()
        });
        assert!(
            resolve_spec(&daytona)
                .unwrap_err()
                .contains("network.mode=allow")
        );
    }

    #[test]
    fn modal_is_offered_only_at_development_grade() {
        assert!(managed_provider_offered("modal", DeploymentGrade::Dev));
        assert!(!managed_provider_offered("modal", DeploymentGrade::Prod));
        assert!(managed_provider_offered("daytona", DeploymentGrade::Prod));

        let resolved = resolve_spec(&profile(SandboxTargetSpec::managed("modal"))).unwrap();
        assert!(validate_target_available_for_grade(&resolved, DeploymentGrade::Dev).is_ok());
        assert!(
            validate_target_available_for_grade(&resolved, DeploymentGrade::Prod)
                .unwrap_err()
                .contains("development grade")
        );
    }

    #[test]
    fn default_must_name_a_profile() {
        let set = SandboxPolicy {
            mode: None,
            default: "missing".to_string(),
            templates: BTreeMap::from([(
                "scratch".to_string(),
                profile(SandboxTargetSpec::vfs("bashkit")),
            )]),
        };
        assert!(
            validate_sandbox_policy(&set)
                .unwrap_err()
                .contains("does not name")
        );
    }
}
