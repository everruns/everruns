//! Shared runtime ownership and bounded grants. Management authority is separate.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Ownership or consumer scope. Ownership determines lifetime, not permission to use a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ResourceScope {
    /// A single session, subject to its verified invocation identity.
    Session,
    /// A service agent, matched against the actual responder.
    Agent,
    /// An end user or service virtual user, matched against the invocation subject.
    VirtualUser,
    /// The current organization; runtime use requires an explicit grant.
    Organization,
}

/// Invocation identity under which a credential may be used; identities never fall back to each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum CredentialUsage {
    /// The verified invoking end user in an eligible private session.
    User,
    /// The authorized service identity of the actual responding agent.
    Service,
}

/// Derived from a persisted invocation, never from tool arguments.
#[derive(Debug, Clone)]
pub struct ResourceActor {
    pub org_id: i64,
    pub session_id: Uuid,
    pub agent_id: Option<Uuid>,
    pub virtual_user_id: Option<Uuid>,
    pub service_user_id: Option<Uuid>,
    pub usage: CredentialUsage,
    pub private_access: bool,
}

impl ResourceActor {
    pub fn subject(&self) -> Option<Uuid> {
        match self.usage {
            CredentialUsage::User if self.private_access => self.virtual_user_id,
            CredentialUsage::User => None,
            CredentialUsage::Service => self.service_user_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceOwner {
    pub org_id: i64,
    pub scope: ResourceScope,
    pub owner_id: Option<Uuid>,
    /// Session grants retain the exact subject and usage that created them.
    pub subject_id: Option<Uuid>,
    pub usage: CredentialUsage,
}

impl ResourceOwner {
    pub fn permits_owner(&self, actor: &ResourceActor) -> bool {
        if self.org_id != actor.org_id || self.usage != actor.usage {
            return false;
        }
        match self.scope {
            ResourceScope::Organization => false,
            ResourceScope::Agent => {
                actor.usage == CredentialUsage::Service
                    && self.owner_id.is_some()
                    && self.owner_id == actor.agent_id
            }
            ResourceScope::VirtualUser => {
                self.owner_id.is_some() && self.owner_id == actor.subject()
            }
            ResourceScope::Session => {
                self.owner_id == Some(actor.session_id)
                    && (self.subject_id == actor.subject()
                        || (self.usage == CredentialUsage::Service && self.subject_id.is_none()))
                    && (actor.usage == CredentialUsage::Service || actor.subject().is_some())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceGrant {
    pub org_id: i64,
    pub consumer_scope: ResourceScope,
    pub consumer_id: Option<Uuid>,
    pub subject_id: Option<Uuid>,
    pub usage: CredentialUsage,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl ResourceGrant {
    /// An organization grant explicitly opts all eligible service agents in.
    pub fn permits(&self, actor: &ResourceActor, now: DateTime<Utc>) -> bool {
        if self.org_id != actor.org_id
            || self.usage != actor.usage
            || self.revoked_at.is_some()
            || self.expires_at.is_some_and(|expiry| expiry <= now)
            || (actor.usage == CredentialUsage::User && actor.subject().is_none())
        {
            return false;
        }
        match self.consumer_scope {
            ResourceScope::Organization => {
                self.consumer_id.is_none()
                    && actor.usage == CredentialUsage::Service
                    && actor.agent_id.is_some()
                    && actor.service_user_id.is_some()
            }
            ResourceScope::Agent => {
                actor.usage == CredentialUsage::Service
                    && self.consumer_id.is_some()
                    && self.consumer_id == actor.agent_id
            }
            ResourceScope::VirtualUser => {
                self.consumer_id.is_some() && self.consumer_id == actor.subject()
            }
            ResourceScope::Session => {
                self.consumer_id == Some(actor.session_id) && self.subject_id == actor.subject()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actor() -> ResourceActor {
        ResourceActor {
            org_id: 1,
            session_id: Uuid::new_v4(),
            agent_id: Some(Uuid::new_v4()),
            virtual_user_id: Some(Uuid::new_v4()),
            service_user_id: Some(Uuid::new_v4()),
            usage: CredentialUsage::User,
            private_access: true,
        }
    }

    #[test]
    fn personal_ownership_is_exact_and_never_uses_management_lineage() {
        let alice = actor();
        let owner = ResourceOwner {
            org_id: 1,
            scope: ResourceScope::VirtualUser,
            owner_id: alice.virtual_user_id,
            subject_id: None,
            usage: CredentialUsage::User,
        };
        assert!(owner.permits_owner(&alice));
        let mut bob = alice.clone();
        bob.virtual_user_id = Some(Uuid::new_v4());
        assert!(!owner.permits_owner(&bob));
        bob.virtual_user_id = alice.virtual_user_id;
        bob.org_id = 2;
        assert!(!owner.permits_owner(&bob));
        bob.org_id = 1;
        bob.private_access = false;
        assert!(!owner.permits_owner(&bob));
        bob.private_access = true;
        bob.usage = CredentialUsage::Service;
        assert!(!owner.permits_owner(&bob));
    }

    #[test]
    fn sharing_a_session_does_not_share_its_personal_credentials() {
        let alice = actor();
        let owner = ResourceOwner {
            org_id: 1,
            scope: ResourceScope::Session,
            owner_id: Some(alice.session_id),
            subject_id: alice.virtual_user_id,
            usage: CredentialUsage::User,
        };
        assert!(owner.permits_owner(&alice));
        let mut bob = alice.clone();
        bob.virtual_user_id = Some(Uuid::new_v4());
        assert!(!owner.permits_owner(&bob));
    }

    #[test]
    fn org_access_is_explicit_expiring_and_service_only() {
        let mut actor = actor();
        let owner = ResourceOwner {
            org_id: 1,
            scope: ResourceScope::Organization,
            owner_id: None,
            subject_id: None,
            usage: CredentialUsage::Service,
        };
        actor.usage = CredentialUsage::Service;
        assert!(!owner.permits_owner(&actor));
        let now = Utc::now();
        let mut grant = ResourceGrant {
            org_id: 1,
            consumer_scope: ResourceScope::Organization,
            consumer_id: None,
            subject_id: None,
            usage: CredentialUsage::Service,
            expires_at: None,
            revoked_at: None,
        };
        assert!(grant.permits(&actor, now));
        grant.expires_at = Some(now);
        assert!(!grant.permits(&actor, now));
        grant.expires_at = None;
        grant.revoked_at = Some(now);
        assert!(!grant.permits(&actor, now));
        grant.revoked_at = None;
        actor.usage = CredentialUsage::User;
        assert!(!grant.permits(&actor, now));
    }
}

/// Provider limits must be enforced by the adapter before a token is exposed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case", deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ResourceConstraints {
    Github {
        repositories: Vec<String>,
        permissions: std::collections::BTreeMap<String, String>,
    },
}
impl ResourceConstraints {
    pub fn validate(&self) -> bool {
        match self {
            Self::Github {
                repositories,
                permissions,
            } => {
                !repositories.is_empty()
                    && repositories.len() <= 100
                    && permissions.len() <= 32
                    && repositories.iter().all(|r| {
                        r.len() <= 255
                            && r.split_once('/').is_some_and(|(o, n)| {
                                !o.is_empty()
                                    && !n.is_empty()
                                    && !n.contains('/')
                                    && r.chars().all(|c| {
                                        c.is_ascii_alphanumeric()
                                            || matches!(c, '/' | '.' | '_' | '-')
                                    })
                            })
                    })
                    && !permissions.is_empty()
                    && permissions.iter().all(|(k, v)| {
                        k.len() <= 64
                            && k.chars().all(|c| c.is_ascii_lowercase() || c == '_')
                            && matches!(v.as_str(), "read" | "write")
                    })
            }
        }
    }
    pub fn permits_github(&self, request: &crate::runtime::GitHubAccess) -> bool {
        match self {
            Self::Github {
                repositories,
                permissions,
            } => {
                self.validate()
                    && !request.repositories.is_empty()
                    && !request.permissions.is_empty()
                    && request.repositories.iter().all(|r| {
                        repositories
                            .iter()
                            .any(|allowed| allowed.eq_ignore_ascii_case(r))
                    })
                    && request.permissions.iter().all(|(key, level)| {
                        permissions.get(key).is_some_and(|allowed| {
                            allowed == level || (allowed == "write" && level == "read")
                        })
                    })
            }
        }
    }
}

#[cfg(test)]
mod constraint_tests {
    use super::*;
    #[test]
    fn github_limits_do_not_allow_widening_or_unbounded_tokens() {
        let limits = ResourceConstraints::Github {
            repositories: vec!["acme/app".into()],
            permissions: std::collections::BTreeMap::from([
                ("contents".into(), "read".into()),
                ("pull_requests".into(), "write".into()),
            ]),
        };
        let mut request = crate::runtime::GitHubAccess {
            acts_as: crate::runtime::McpServerActsAs::Service,
            repositories: vec!["Acme/App".into()],
            permissions: std::collections::BTreeMap::from([("contents".into(), "read".into())]),
        };
        assert!(limits.permits_github(&request));
        request.repositories.push("acme/other".into());
        assert!(!limits.permits_github(&request));
        request.repositories.pop();
        request
            .permissions
            .insert("contents".into(), "write".into());
        assert!(!limits.permits_github(&request));
        request.repositories.clear();
        assert!(!limits.permits_github(&request));
    }
    #[test]
    fn personal_session_grant_never_admits_another_participant() {
        let alice = Uuid::now_v7();
        let session = Uuid::now_v7();
        let mut actor = ResourceActor {
            org_id: 1,
            session_id: session,
            agent_id: None,
            virtual_user_id: Some(alice),
            service_user_id: None,
            usage: CredentialUsage::User,
            private_access: true,
        };
        let grant = ResourceGrant {
            org_id: 1,
            consumer_scope: ResourceScope::Session,
            consumer_id: Some(session),
            subject_id: Some(alice),
            usage: CredentialUsage::User,
            expires_at: None,
            revoked_at: None,
        };
        assert!(grant.permits(&actor, Utc::now()));
        actor.virtual_user_id = Some(Uuid::now_v7());
        assert!(!grant.permits(&actor, Utc::now()));
        actor.virtual_user_id = Some(alice);
        actor.private_access = false;
        assert!(!grant.permits(&actor, Utc::now()));
    }
}
