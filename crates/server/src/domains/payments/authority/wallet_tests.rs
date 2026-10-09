//! EVE-1187: a human user's wallet is spendable only from an interactive,
//! user-initiated turn. Scheduled, trigger-driven, and otherwise unattended
//! turns must not inherit the session owner's wallet authority, while
//! explicit session/org/agent/endpoint policies keep working for them.

use super::*;
use crate::storage::models::{CreateOrganizationRow, CreateSessionRow, CreateUserRow};
use crate::storage::{CreatePaymentAccountRow, CreatePaymentPolicyRow};

struct Fixture {
    db: Arc<StorageBackend>,
    org_id: i64,
    owner: uuid::Uuid,
    session_id: SessionId,
}

async fn fixture() -> Fixture {
    let db = Arc::new(StorageBackend::test_database());
    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: "org_00000000000000000000000000001187".to_string(),
            name: "Wallet provenance".to_string(),
            created_by: None,
        })
        .await
        .expect("create org");
    let owner = db
        .create_user(CreateUserRow {
            email: "owner@example.com".to_string(),
            name: "Owner".to_string(),
            avatar_url: None,
            external_id: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("test".to_string()),
            auth_provider_id: None,
        })
        .await
        .expect("create user")
        .id;
    db.ensure_membership(owner, org.org_id, "owner")
        .await
        .expect("membership");
    let session_id = db
        .create_session(CreateSessionRow {
            playground_user_id: None,
            trigger_id: None,
            source: crate::records::SessionSource::Api,
            workspace_id: None,
            org_id: org.org_id,
            app_id: None,
            channel_id: None,
            harness_id: None,
            agent_id: None,
            agent_revision: None,
            virtual_user_id: None,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            // The session resolves to a human owner: this is the wallet an
            // unattended turn must not inherit.
            resolved_owner_user_id: Some(owner),
            title: Some("wallet-provenance".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: json!([]),
            tools: json!([]),
            mcp_servers: json!({}),
            system_prompt: None,
            initial_files: json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
        })
        .await
        .expect("create session")
        .id;
    Fixture {
        db,
        org_id: org.org_id,
        owner,
        session_id,
    }
}

impl Fixture {
    /// A wallet plus a policy granting `subject` spend from it.
    async fn grant(&self, subject_type: &str, subject_id: &str) -> uuid::Uuid {
        let account = self
            .db
            .create_payment_account(
                self.org_id,
                CreatePaymentAccountRow {
                    owner_type: "user".to_string(),
                    owner_id: self.owner.to_string(),
                    rail: "x402_base".to_string(),
                    label: format!("{subject_type} wallet"),
                    public_address: None,
                    credential_encrypted: None,
                    metadata: json!({}),
                },
            )
            .await
            .expect("create account");
        self.db
            .create_payment_policy(
                self.org_id,
                CreatePaymentPolicyRow {
                    payment_account_id: account.id,
                    subject_type: subject_type.to_string(),
                    subject_id: subject_id.to_string(),
                    allowed_capabilities: vec!["parallel".to_string()],
                    allowed_hosts: vec!["parallelmpp.dev".to_string()],
                    rail_preference: vec![],
                    max_amount_usd_per_request: Some(1.0),
                    max_amount_usd_per_turn: None,
                    max_amount_usd_per_day: None,
                    require_approval_above_usd: None,
                    metadata: json!({}),
                },
            )
            .await
            .expect("create policy")
            .id
    }

    async fn owner_wallet(&self) -> uuid::Uuid {
        self.grant("user", &self.owner.to_string()).await
    }

    /// An input recorded the way `MessageService::create` records an
    /// authenticated user's own message: the default runtime subject plus the
    /// management user that proved the call.
    async fn interactive_input(&self) -> uuid::Uuid {
        let message = uuid::Uuid::now_v7();
        let subject = self
            .db
            .default_virtual_user(self.org_id, self.owner)
            .await
            .expect("default runtime subject");
        self.db
            .record_runtime_invocation(
                self.org_id,
                self.session_id,
                message,
                Some(subject.id),
                Some(self.owner),
                None,
            )
            .await
            .expect("record interactive input");
        message
    }

    /// An input recorded without a management user: the shape the session
    /// scheduler, agent triggers, app channels, health checks and evals all
    /// produce, because no authenticated human sent it.
    async fn unattended_input(&self) -> uuid::Uuid {
        let message = uuid::Uuid::now_v7();
        self.db
            .record_runtime_invocation(self.org_id, self.session_id, message, None, None, None)
            .await
            .expect("record unattended input");
        message
    }

    fn authority(&self, input_message_id: Option<uuid::Uuid>) -> ServerPaymentAuthority {
        let authority = ServerPaymentAuthority::new(self.db.clone(), None, self.org_id, None);
        match input_message_id {
            Some(id) => authority.bound_to_input_message(id),
            None => authority,
        }
    }

    async fn select(&self, input_message_id: Option<uuid::Uuid>) -> Result<uuid::Uuid> {
        self.authority(input_message_id)
            .select_policy(self.session_id, &paid_request())
            .await
            .map(|selected| selected.policy.id)
    }
}

fn paid_request() -> MachinePaymentRequest {
    MachinePaymentRequest {
        capability: "parallel".into(),
        operation: "search".into(),
        method: PaymentMethod::Post,
        url: "https://parallelmpp.dev/api/search".into(),
        body: None,
        max_amount_usd: 0.01,
        rail_preference: vec![],
        metadata: json!({}),
    }
}

#[tokio::test]
async fn scheduled_turn_cannot_spend_the_owner_wallet() {
    let fx = fixture().await;
    fx.owner_wallet().await;
    let input = fx.unattended_input().await;

    let error = fx.select(Some(input)).await.unwrap_err();
    assert!(
        error.to_string().contains("No active payment policy"),
        "{error}"
    );
}

#[tokio::test]
async fn agent_triggered_turn_cannot_spend_the_owner_wallet() {
    // Agent triggers dispatch through `MessageService` with server-authored
    // `agent_trigger` event metadata and no authenticated user, so the
    // invocation carries no management user, the same as a schedule.
    let fx = fixture().await;
    fx.owner_wallet().await;
    let input = fx.unattended_input().await;

    assert!(fx.select(Some(input)).await.is_err());
}

#[tokio::test]
async fn unbound_or_unknown_provenance_fails_closed_for_the_owner_wallet() {
    let fx = fixture().await;
    fx.owner_wallet().await;

    // No input binding at all: provenance is absent.
    assert!(fx.select(None).await.is_err());
    // A binding to an input this session never recorded proves nothing.
    assert!(fx.select(Some(uuid::Uuid::now_v7())).await.is_err());
    // An interactive input recorded on another session is not this turn's.
    let other = fixture().await;
    let foreign = other.interactive_input().await;
    assert!(fx.select(Some(foreign)).await.is_err());
}

#[tokio::test]
async fn interactive_user_turn_may_spend_the_owner_wallet() {
    let fx = fixture().await;
    let policy = fx.owner_wallet().await;
    let input = fx.interactive_input().await;

    assert_eq!(fx.select(Some(input)).await.unwrap(), policy);
}

#[tokio::test]
async fn explicit_session_and_org_policies_still_apply_to_unattended_turns() {
    let fx = fixture().await;
    fx.owner_wallet().await;
    let session_policy = fx.grant("session", &fx.session_id.to_string()).await;
    let input = fx.unattended_input().await;
    assert_eq!(fx.select(Some(input)).await.unwrap(), session_policy);

    let fx = fixture().await;
    let org_policy = fx.grant("org", "org").await;
    assert_eq!(fx.select(None).await.unwrap(), org_policy);
}

#[test]
fn user_candidate_is_the_proven_initiator_only() {
    let session_id = SessionId::new();
    let agent_id = AgentId::new();
    let agent_public_id = AgentId::new().to_string();
    let channel_public_id = "appchan_0199f0c2d4b17a3e9c1155aa77e30b41".to_string();
    let initiator = uuid::Uuid::now_v7();

    let unattended = subject_candidates(
        session_id,
        Some(agent_id),
        Some(agent_public_id.clone()),
        None,
        None,
        Some(channel_public_id.clone()),
    );
    assert!(!unattended.iter().any(|(kind, _)| *kind == "user"));
    // Agent and endpoint policies do not depend on a human being present.
    assert!(unattended.contains(&("agent", agent_public_id.clone())));
    assert!(unattended.contains(&("agent_channel", channel_public_id.clone())));

    let interactive = subject_candidates(
        session_id,
        Some(agent_id),
        Some(agent_public_id),
        None,
        Some(initiator),
        Some(channel_public_id),
    );
    assert!(interactive.contains(&("user", initiator.to_string())));
}
