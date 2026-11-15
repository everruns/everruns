//! Sandbox Template validation, resolution, and runtime capability mapping.

use crate::domains::sandbox_templates::record::{
    ResolvedSandboxSpec, SandboxContainmentLevel, SandboxContainmentSpec, SandboxDurability,
    SandboxEscalation, SandboxIdleAction, SandboxNetworkPolicy, SandboxPolicy, SandboxPolicyMode,
    SandboxSelection, SandboxTargetKind, SandboxTemplateSpec,
};
use everruns_capabilities::capabilities::SANDBOX_DESKTOP_CAPABILITY_ID;
use everruns_contracts::capability::CapabilityRef;
use everruns_contracts::session_sandbox::SessionSandboxCredentialSource;
use everruns_core::DeploymentGrade;
use serde_json::{Map, Value, json};

use super::target_options::validate_target_options;

const MAX_SANDBOX_TEMPLATES: usize = 16;
const MAX_PROVIDER_OPTIONS_BYTES: usize = 32 * 1024;
const MAX_BOOTSTRAP_COMMANDS: usize = 32;
const MAX_BOOTSTRAP_COMMAND_BYTES: usize = 8 * 1024;

/// Capabilities that provide the `computer` tool. A Sandbox Template with a
/// desktop replaces them with `sandbox_desktop`.
const COMPUTER_USE_CAPABILITY_IDS: &[&str] = &["computer_use", SANDBOX_DESKTOP_CAPABILITY_ID];

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
const MANAGED_PROVIDERS: &[&str] = &["boxd", "daytona", "e2b", "kubernetes", "modal"];

/// Managed providers that enforce `containment.network` themselves, from the
/// `network` entry the server writes into their provider config.
/// `kubernetes` enforces deny and IP allowlists with a NetworkPolicy and
/// refuses domain allowlists (`kubernetes_egress`).
const PROVIDER_ENFORCED_NETWORK: &[&str] = &["boxd", "daytona", "e2b", "kubernetes", "modal"];

/// Managed providers that run on the deployment's own infrastructure: their
/// credential source is `platform`, never a user or organization account.
pub(crate) const PLATFORM_PROVIDERS: &[&str] = &["kubernetes"];

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
    if is_managed(authored, "boxd") {
        boxd_egress(&containment)?;
    }
    if is_managed(authored, "kubernetes") {
        kubernetes_egress(&containment)?;
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
    if let Some(seconds) = authored.lifecycle.delete_after_seconds
        && !(3_600..=90 * 86_400).contains(&seconds)
    {
        return Err(
            "lifecycle.delete_after_seconds must be between 3600 and 7776000, or null to keep paused Sandboxes"
                .to_string(),
        );
    }

    let mut target = authored.target.clone();
    let platform = target
        .provider
        .as_deref()
        .is_some_and(|provider| PLATFORM_PROVIDERS.contains(&provider));
    if target.kind == SandboxTargetKind::Managed
        && platform
        && target.credential.source == SessionSandboxCredentialSource::None
    {
        // The deployment runs these itself; there is no account to pick.
        target.credential.source = SessionSandboxCredentialSource::Platform;
    } else if target.kind == SandboxTargetKind::Managed
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

/// The organization account a `session_user` template uses when the Session
/// has no user to hold a provider account.
///
/// A trigger, schedule or channel ingress starts its Session without a person,
/// so "the Session user's account" names nobody. The organization's account for
/// the provider is the only owner left, and it is used only when it is the one
/// account for that provider: with several, picking one would silently bill an
/// arbitrary account, so the template has to name it instead.
pub(crate) fn sole_organization_account<T>(
    provider: &str,
    accounts: impl IntoIterator<Item = T>,
    provider_of: impl Fn(&T) -> &str,
) -> Result<T, String> {
    let mut matching = accounts
        .into_iter()
        .filter(|account| provider_of(account) == provider);
    let first = matching.next();
    let others = matching.count();
    match (first, others) {
        (Some(account), 0) => Ok(account),
        (None, _) => Err(format!(
            "This Sandbox Template uses the Session user's {provider} account, but this \
             session has no user (it was started by a trigger or channel) and the \
             organization has no {provider} account. Add an organization {provider} \
             account, or set the template's account to one."
        )),
        (Some(_), others) => Err(format!(
            "This Sandbox Template uses the Session user's {provider} account, but this \
             session has no user (it was started by a trigger or channel) and the \
             organization has {} {provider} accounts. Set the template's account to the \
             one it should use.",
            others + 1
        )),
    }
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

    let platform = target
        .provider
        .as_deref()
        .is_some_and(|provider| PLATFORM_PROVIDERS.contains(&provider));
    if platform != (credential.source == SessionSandboxCredentialSource::Platform) {
        return Err(if platform {
            "this target runs on the deployment's own infrastructure; its credential source is platform"
                .to_string()
        } else {
            "credential source platform is only for targets the deployment runs itself".to_string()
        });
    }
    match credential.source {
        SessionSandboxCredentialSource::None => {
            Err("managed target requires a credential source".to_string())
        }
        SessionSandboxCredentialSource::Platform => {
            if credential.connection_id.is_some() {
                return Err("platform credentials cannot name a connection".to_string());
            }
            Ok(())
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

/// Isolated sandboxes enforce egress with a NetworkPolicy, which can allow IP
/// ranges but not domains. A domain allowlist is refused here rather than
/// widened to open egress at runtime.
fn kubernetes_egress(containment: &SandboxContainmentSpec) -> Result<(), String> {
    let rules = everruns_contracts::runtime::sandbox_egress::sandbox_egress_rules(
        Some(&provider_network_config(&containment.network)),
        None,
    )
    .map_err(|error| format!("isolated sandbox egress: {error}"))?;
    match rules.mode {
        everruns_contracts::runtime::sandbox_egress::SandboxEgressMode::Allowlist {
            domains,
            ..
        } if !domains.is_empty() => Err(format!(
            "isolated sandboxes cannot enforce a domain allowlist yet ({}); use network allow or deny, or allow IP ranges only",
            domains.join(", ")
        )),
        _ => Ok(()),
    }
}

fn is_modal(profile: &SandboxTemplateSpec) -> bool {
    is_managed(profile, "modal")
}

fn is_managed(profile: &SandboxTemplateSpec, provider: &str) -> bool {
    profile.target.kind == SandboxTargetKind::Managed
        && profile.target.provider.as_deref() == Some(provider)
}

/// The allowlist boxd would hold for the template's network policy, checked
/// with boxd's own rules (no IPv6, at most 256 entries) so a template cannot
/// save what machine creation would refuse.
fn boxd_egress(containment: &SandboxContainmentSpec) -> Result<(), String> {
    let network = provider_network_config(&containment.network);
    let rules =
        everruns_contracts::runtime::sandbox_egress::sandbox_egress_rules(Some(&network), None)
            .map_err(|error| format!("boxd egress: {error}"))?;
    everruns_integrations::boxd::egress::entries(&rules)
        .map(|_| ())
        .map_err(|error| format!("boxd egress: {error}"))
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
        provider_network_config(&containment.network),
    );
    if let Some(connections) = profile.target.options.get("inject_connections") {
        egress.insert("inject_connections".to_string(), connections.clone());
    }
    everruns_integrations::modal::egress::EgressSpec::from_json(&Value::Object(egress))
        .map(|_| ())
        .map_err(|error| format!("modal egress: {error}"))
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
        // An IP range (`10.0.0.0/8`) is the one entry that may hold a slash:
        // the egress rules carry it as a CIDR, and isolated sandboxes accept
        // nothing else.
        if allowed_hosts.iter().any(|host| {
            host.is_empty()
                || host.len() > 253
                || host.contains('@')
                || (host.contains('/') && !is_ip_range(host))
        }) {
            return Err(
                "network allowlist entries must be hostnames or IP ranges, not URLs or credentials"
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn is_ip_range(entry: &str) -> bool {
    let Some((address, prefix)) = entry.split_once('/') else {
        return false;
    };
    let Ok(address) = address.parse::<std::net::IpAddr>() else {
        return false;
    };
    let max = if address.is_ipv4() { 32 } else { 128 };
    prefix.parse::<u8>().is_ok_and(|bits| bits <= max)
}

/// Replace every legacy compute capability with the one selected by the pinned
/// profile. Non-compute capabilities keep their original layering and order.
///
/// `inherited` holds the broader layers (harness and agent) when
/// `capabilities` is only the session layer. A Bashkit profile keeps the
/// shell's `enable_http` opt-in from either list: the pinned profile picks
/// where commands run, not whether the agent may use curl, and the session
/// layer overrides the agent layer by capability id.
pub fn apply_sandbox_to_capabilities(
    capabilities: &[CapabilityRef],
    inherited: &[CapabilityRef],
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
    let selected = capability_for_sandbox(environment);
    let desktop = (selected.id() == "session_sandbox"
        && selected.config_value()["provider_config"]
            .get("desktop")
            .is_some_and(|desktop| desktop.as_bool() != Some(false)))
    .then(|| {
        CapabilityRef::with_config(
            SANDBOX_DESKTOP_CAPABILITY_ID,
            selected.config_value().clone(),
        )
    });
    if desktop.is_some() {
        // The Sandbox desktop is the agent's one display (`computer` is an
        // exclusive group), so it replaces any other computer-use backend.
        result.retain(|capability| !COMPUTER_USE_CAPABILITY_IDS.contains(&capability.id()));
    }
    let mut selected = selected;
    if environment.target.kind == SandboxTargetKind::Vfs
        && bashkit_http_opt_in(capabilities).or_else(|| bashkit_http_opt_in(inherited))
            == Some(true)
    {
        selected = CapabilityRef::with_config("bashkit_shell", json!({"enable_http": true}));
    }
    result.push(selected);
    result.extend(desktop);
    result
}

/// The `enable_http` value of the last Bashkit shell entry, if any sets it.
fn bashkit_http_opt_in(capabilities: &[CapabilityRef]) -> Option<bool> {
    capabilities
        .iter()
        .rev()
        .find(|capability| matches!(capability.id(), "bashkit_shell" | "virtual_bash"))
        .and_then(|capability| capability.config_value().get("enable_http"))
        .and_then(serde_json::Value::as_bool)
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
            if profile
                .target
                .provider
                .as_deref()
                .is_some_and(|provider| PROVIDER_ENFORCED_NETWORK.contains(&provider))
            {
                // Server-written: `network` is not a caller option.
                provider_config.insert(
                    "network".to_string(),
                    provider_network_config(&profile.containment.network),
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

fn provider_network_config(network: &SandboxNetworkPolicy) -> Value {
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
            // Modal (`modal_egress`), boxd (`boxd_egress`), Daytona and E2B
            // (their `network` modules) enforce deny and allowlists themselves.
            if !provider.is_some_and(|provider| PROVIDER_ENFORCED_NETWORK.contains(&provider))
                && containment.network != SandboxNetworkPolicy::Allow
            {
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
mod tests;
