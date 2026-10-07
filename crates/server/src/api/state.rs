// Shared state for API modules.
//
// Decision: one `ApiState`, built once at startup, replaces the per-module
// `AppState` structs that each carried their own copy of `db` + `auth` and
// assembled their own `Ctx` (with a different subset of services each time).
// Every module that serves domain commands now builds the same fully wired
// `Ctx`, the one MCP and the generic command adapter use, so a command
// behaves the same whichever transport calls it.
//
// SECURITY: the context template holds every service but no caller. Its
// placeholder caller is a member of no organization and is replaced on every
// `ctx()` call; the template itself is never handed out.

use std::sync::Arc;

use everruns_core::Caller;

use super::common::impl_auth_state;
use super::dispatch::impl_dispatchable;
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::Ctx;
use crate::services::CapabilityService;
use crate::storage::StorageBackend;
use crate::storage::encryption::EncryptionService;

#[derive(Clone)]
pub struct ApiState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
    template: Arc<Ctx>,
}

impl ApiState {
    /// `services` supplies every service a command may use. Its caller,
    /// feature flags and acting session are ignored.
    pub fn new(services: Ctx, auth: AuthState) -> Self {
        let template = Ctx {
            caller: nobody(),
            acting_for_session: None,
            ..services
        };
        Self {
            db: template.db.clone(),
            encryption: template.encryption.clone(),
            capability_service: template.capability_service.clone(),
            auth,
            template: Arc::new(template),
        }
    }

    /// Share the services the MCP endpoint already wires, so a command sees
    /// the same context over REST and MCP.
    pub fn from_mcp(state: &super::mcp_endpoint::AppState) -> Self {
        Self::new(
            super::mcp_endpoint::domain_context(nobody(), state),
            state.auth.clone(),
        )
    }

    /// State with storage and capabilities only, for embedders and tests that
    /// serve a few modules without the full service graph.
    pub fn basic(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
    ) -> Self {
        let services = Ctx::new(
            nobody(),
            db,
            capability_service,
            encryption,
            auth.permission_resolver.clone(),
        );
        Self::new(services, auth)
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        auth: AuthState,
    ) -> Self {
        let capabilities = Arc::new(CapabilityService::new(db.clone(), None));
        Self::basic(db, encryption, capabilities, auth)
    }

    /// The domain context for a request in `org`.
    pub fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx {
            caller: Caller::from(org),
            feature_flags: org.feature_flags.clone(),
            // `auth` owns the resolver, so a SaaS override set on it applies.
            permission_resolver: self.auth.permission_resolver.clone(),
            ..(*self.template).clone()
        }
    }
}

impl_auth_state!(ApiState);
impl_dispatchable!(ApiState);

/// A caller with no organization and no privileges.
fn nobody() -> Caller {
    Caller {
        org_id: 0,
        org_public_id: String::new(),
        user_id: None,
        role: everruns_core::OrgRole::Member,
        is_platform_user: false,
        is_internal: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::config::AuthConfig;
    use crate::domains::common::all_feature_flags_for_test;
    use everruns_core::OrgRole;

    fn org() -> ResolvedOrg {
        ResolvedOrg {
            org_id: 42,
            public_id: "org_42".to_string(),
            name: "Acme".to_string(),
            user_id: None,
            role: OrgRole::Admin,
            is_platform_user: false,
            feature_flags: all_feature_flags_for_test(),
        }
    }

    #[test]
    fn ctx_acts_as_the_requesting_org_never_the_template_caller() {
        let db = Arc::new(StorageBackend::test_database());
        // A privileged template caller must not leak into request contexts.
        let services = Ctx::minimal_for_test(Caller::internal(7), db.clone(), None);
        let state = ApiState::new(services, AuthState::builtin(AuthConfig::default(), db));
        assert!(!state.template.caller.is_internal);

        let ctx = state.ctx(&org());
        assert_eq!(ctx.caller.org_id, 42);
        assert_eq!(ctx.caller.role, OrgRole::Admin);
        assert!(!ctx.caller.is_internal);
        assert!(ctx.acting_for_session.is_none());
        assert_eq!(ctx.feature_flags, all_feature_flags_for_test());
    }
}
