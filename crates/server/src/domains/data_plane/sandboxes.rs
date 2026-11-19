//! A v2 org's managed sandboxes (data plane v2, step 11.3).
//!
//! Decisions:
//! - The control plane pins a session's primary sandbox at create, as for
//!   any org, and sends it with the data plane's create as a
//!   `session_sandbox` capability entry. The data plane adds it to the
//!   session's agent, so an agent's sandbox is the session's pinned one and
//!   never one a definition names.
//! - The provider key goes as a sealed org secret
//!   ([`sandbox_key_secret`]), resolved here through the same
//!   session-authorized lookup a v1 worker uses
//!   (`get_sandbox_connection_token`, THREAT[TM-DAYTONA-013]). It is
//!   sealed again only when it changed, so a session create does not move
//!   the org's config when nothing did.
//! - An agent's own `sandbox_policy` and sandbox capabilities stay with the
//!   control plane: the published package the data plane runs carries
//!   neither (a definition naming the sandbox would bypass the pin).

use everruns_contracts::dataplane::sandbox_key_secret;
use everruns_contracts::session_sandbox::SessionSandboxCredentialSource;
use everruns_contracts::typed_id::SessionId;
use everruns_core::UserConnectionResolver as _;
use serde_json::Value;

use super::{ControlPlane, providers, put_secret};
use crate::domains::sandbox_templates::record::SandboxTargetKind;
use crate::domains::sandbox_templates::resolution::capability_for_sandbox;
use crate::storage::DbConnectionResolver;

/// Package fields that choose a sandbox. Engine's builder refuses a policy
/// it is not bound to, and the data plane takes the session's pinned one.
const POLICY_FIELDS: &[&str] = &["sandbox_policy", "environments"];
/// Capabilities that are a sandbox; the pinned one replaces them.
const SANDBOX_CAPABILITIES: &[&str] = &["session_sandbox", "sandbox_desktop"];

/// Take what chooses a sandbox out of an agent's published package.
pub fn strip(package: &mut Value) {
    if let Some(object) = package.as_object_mut() {
        for field in POLICY_FIELDS {
            object.remove(*field);
        }
    }
    // `get_mut`, not indexing: indexing a missing field writes a null,
    // which no manifest reads as a list.
    if let Some(capabilities) = package
        .get_mut("capabilities")
        .and_then(Value::as_array_mut)
    {
        capabilities.retain(|capability| {
            let id = capability["ref"].as_str().or_else(|| capability.as_str());
            !id.is_some_and(|id| SANDBOX_CAPABILITIES.contains(&id))
        });
    }
}

/// The managed sandbox pinned for `session`, as the capability entry the
/// data plane adds to its agent, its provider key sealed for the org.
/// `None`: the session has no managed sandbox.
pub async fn pinned(
    cp: &ControlPlane,
    org_id: i64,
    session: SessionId,
) -> anyhow::Result<Option<Value>> {
    let Some(pinned) = cp.db.get_primary_sandbox(session).await? else {
        return Ok(None);
    };
    let spec = &pinned.spec;
    if spec.target.kind != SandboxTargetKind::Managed {
        return Ok(None);
    }
    let provider = spec
        .target
        .provider
        .clone()
        .ok_or_else(|| anyhow::anyhow!("session {session}'s sandbox names no provider"))?;
    let credential = &spec.target.credential;
    if !matches!(
        credential.source,
        SessionSandboxCredentialSource::None | SessionSandboxCredentialSource::Platform
    ) {
        let resolver = DbConnectionResolver::new(
            cp.db.as_ref().clone(),
            cp.encryption()?.clone(),
            None,
            std::sync::Arc::new(
                everruns_core::host::DirectEgressService::for_runtime_traffic_from_env(),
            ),
        );
        let key = resolver
            .get_sandbox_connection_token(session, &provider, credential)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("no {provider} account for session {session}'s sandbox")
            })?;
        let secret = sandbox_key_secret(
            &provider,
            credential.virtual_user_id,
            credential.connection_id,
        );
        if providers::sealed_key(cp, org_id, &secret).await?.as_deref() != Some(key.as_bytes()) {
            put_secret(cp, org_id, &secret, key.as_bytes()).await?;
        }
    }
    Ok(Some(serde_json::to_value(capability_for_sandbox(spec))?))
}
