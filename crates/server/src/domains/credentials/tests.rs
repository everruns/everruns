use super::{management, service};
use crate::domains::common::Ctx;
use crate::storage::runtime_identity::VerifiedRuntimeIdentity;
use crate::storage::{CreateSessionRow, EncryptionService, StorageBackend};
use everruns_contracts::runtime::credential_services::*;
use everruns_contracts::runtime::resource_access::*;
use everruns_contracts::runtime::{SecretAuth, SecretBinding};
use everruns_contracts::typed_id::{PrincipalId, SessionId};
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

fn context(db: Arc<StorageBackend>) -> Ctx {
    Ctx::minimal_for_test(
        Caller::internal(DEFAULT_ORG_ID),
        db,
        Some(Arc::new(
            EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[])
                .unwrap(),
        )),
    )
}
async fn invoke(
    db: &StorageBackend,
    session: SessionId,
    name: Option<&str>,
) -> CredentialInvocation {
    let subject = if let Some(name) = name {
        Some(
            db.resolve_runtime_identity(VerifiedRuntimeIdentity {
                org_id: DEFAULT_ORG_ID,
                provider: "oidc".into(),
                realm: "credential-tests".into(),
                subject: name.into(),
                name: name.into(),
                avatar_url: None,
                management_user_id: None,
            })
            .await
            .unwrap()
            .id,
        )
    } else {
        None
    };
    let input = Uuid::now_v7();
    db.record_runtime_invocation(DEFAULT_ORG_ID, session, input, subject, None, None)
        .await
        .unwrap();
    CredentialInvocation {
        session_id: session,
        input_message_id: input,
    }
}
async fn new_session(db: &StorageBackend) -> SessionId {
    db.create_session(CreateSessionRow {
        org_id: DEFAULT_ORG_ID,
        owner_principal_id: PrincipalId::from_seed(1),
        title: Some("Credential capture".into()),
        ..Default::default()
    })
    .await
    .unwrap()
    .id
}
fn plan(scope: ResourceScope) -> CapturePlan {
    CapturePlan {
        usage: CredentialUsage::User,
        source_url: "https://mail.example/oauth/token".into(),
        arguments: json!({"url":"https://mail.example/oauth/token","method":"POST"}),
        captures: vec![CaptureSpec {
            name: "MAIL_TOKEN".into(),
            from: "/access_token".into(),
            scope,
            binding: SecretBinding {
                url: "https://mail.example/api".into(),
                auth: SecretAuth::Bearer,
            },
            expires_in_from: None,
            expires_at_from: None,
        }],
    }
}

#[tokio::test]
async fn session_capture_is_atomic_encrypted_write_only_and_subject_bound() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = context(db.clone());
    let session = new_session(&db).await;
    let alice = invoke(&db, session, Some("alice")).await;
    let preparation: CapturePreparation = serde_json::from_value(
        service::execute(&ctx, alice, "prepare", json!(plan(ResourceScope::Session)))
            .await
            .unwrap(),
    )
    .unwrap();
    let CapturePreparation::Ready { ticket } = preparation else {
        panic!("session capture should be authorized")
    };
    let plaintext = "captured-token-must-never-be-listed";
    service::execute(&ctx,alice,"commit",json!({"ticket":ticket,"values":[{"name":"MAIL_TOKEN","value":plaintext,"expires_at":null}],"result":{"body":"$MAIL_TOKEN"}})).await.unwrap();
    let listed = service::execute(&ctx, alice, "list", json!({"usage":"user"}))
        .await
        .unwrap();
    assert!(!listed.to_string().contains(plaintext));
    assert_eq!(listed[0]["name"], "MAIL_TOKEN");
    let raw: Vec<u8> = sqlx::query_scalar(
        "SELECT value_encrypted FROM credential_bindings WHERE env_name='MAIL_TOKEN'",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains(plaintext));
    let resolved = service::execute(
        &ctx,
        alice,
        "resolve",
        json!({"usage":"user","names":["MAIL_TOKEN"]}),
    )
    .await
    .unwrap();
    assert_eq!(resolved[0]["value"], plaintext);
    let bob = invoke(&db, session, Some("bob")).await;
    assert!(
        service::execute(
            &ctx,
            bob,
            "resolve",
            json!({"usage":"user","names":["MAIL_TOKEN"]})
        )
        .await
        .is_err()
    );
    let foreign = CredentialInvocation {
        session_id: SessionId::new(),
        ..alice
    };
    assert!(
        service::execute(&ctx, foreign, "list", json!({"usage":"user"}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn persistent_capture_requires_exact_one_off_consent_and_replays_are_safe() {
    use everruns_contracts::tool_approval_types::{
        StoredToolApproval, one_off_decision_storage_key,
    };
    let db = Arc::new(StorageBackend::test_database());
    let ctx = context(db.clone());
    let session = new_session(&db).await;
    let alice = invoke(&db, session, Some("alice")).await;
    let request = plan(ResourceScope::VirtualUser);
    let preparation: CapturePreparation = serde_json::from_value(
        service::execute(&ctx, alice, "prepare", json!(request))
            .await
            .unwrap(),
    )
    .unwrap();
    let CapturePreparation::ConsentRequired {
        ticket, summary, ..
    } = preparation
    else {
        panic!("persistent capture requires consent")
    };
    assert_eq!(summary["credentials"][0]["owner_label"], "alice");
    assert!(
        service::execute(
            &ctx,
            alice,
            "commit",
            json!({"ticket":ticket,"values":[],"result":{}})
        )
        .await
        .is_err()
    );
    // A general always-allow rule does not authorize credential persistence.
    db.upsert_session_key_value(crate::storage::UpsertSessionKeyValue {
        session_id: session,
        key: everruns_contracts::tool_approval_types::always_decision_storage_key("web_fetch"),
        value: serde_json::to_string(&StoredToolApproval::always(
            "web_fetch",
            true,
            chrono::Utc::now(),
        ))
        .unwrap(),
    })
    .await
    .unwrap();
    assert_eq!(
        service::execute(&ctx, alice, "prepare", json!(request))
            .await
            .unwrap()["status"],
        "consent_required"
    );
    let fingerprint = format!("credential:{ticket}");
    db.upsert_session_key_value(crate::storage::UpsertSessionKeyValue {
        session_id: session,
        key: one_off_decision_storage_key(&fingerprint),
        value: serde_json::to_string(&StoredToolApproval::one_off(
            "web_fetch",
            &fingerprint,
            true,
            chrono::Utc::now(),
        ))
        .unwrap(),
    })
    .await
    .unwrap();
    assert_eq!(
        service::execute(&ctx, alice, "prepare", json!(request))
            .await
            .unwrap()["status"],
        "ready"
    );
    assert!(
        service::execute(&ctx, alice, "prepare", json!(request))
            .await
            .is_err()
    );
    service::execute(&ctx,alice,"commit",json!({"ticket":ticket,"values":[{"name":"MAIL_TOKEN","value":"persistent-secret-sentinel","expires_at":null}],"result":{"body":"$MAIL_TOKEN"}})).await.unwrap();
    let next = invoke(&db, new_session(&db).await, Some("alice")).await;
    assert_eq!(
        service::execute(
            &ctx,
            next,
            "resolve",
            json!({"usage":"user","names":["MAIL_TOKEN"]})
        )
        .await
        .unwrap()[0]["value"],
        "persistent-secret-sentinel"
    );
    assert_eq!(
        service::execute(&ctx, alice, "prepare", json!(request))
            .await
            .unwrap()["status"],
        "completed"
    );
}

#[tokio::test]
async fn failed_capture_preserves_all_existing_values_and_revocation_cannot_resurrect() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = context(db.clone());
    let session = new_session(&db).await;
    let alice = invoke(&db, session, Some("alice")).await;
    let p = plan(ResourceScope::Session);
    let prepared = service::execute(&ctx, alice, "prepare", json!(p))
        .await
        .unwrap();
    let ticket = prepared["ticket"].as_str().unwrap();
    assert!(service::execute(&ctx,alice,"commit",json!({"ticket":ticket,"values":[{"name":"WRONG_NAME","value":"do-not-persist","expires_at":null}],"result":{}})).await.is_err());
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM credential_bindings WHERE env_name='MAIL_TOKEN'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(count, 0);
    service::execute(&ctx,alice,"commit",json!({"ticket":ticket,"values":[{"name":"MAIL_TOKEN","value":"original-sentinel","expires_at":null}],"result":{"body":"$MAIL_TOKEN"}})).await.unwrap();
    let id: Uuid =
        sqlx::query_scalar("SELECT id FROM credential_bindings WHERE env_name='MAIL_TOKEN'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    sqlx::query("UPDATE resource_access SET revoked_at=now(),revision=revision+1 WHERE id=$1")
        .bind(id)
        .execute(db.pool())
        .await
        .unwrap();
    assert!(
        service::execute(
            &ctx,
            alice,
            "resolve",
            json!({"usage":"user","names":["MAIL_TOKEN"]})
        )
        .await
        .is_err()
    );
    let raw: Value = service::execute(&ctx, alice, "list", json!({"usage":"user"}))
        .await
        .unwrap();
    assert_eq!(raw, json!([]));
}

#[tokio::test]
async fn org_credentials_are_not_implicitly_available_and_typed_owners_are_enforced() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = context(db.clone());
    let id = management::create(
        &ctx,
        management::CreateScopedCredential {
            owner: management::CredentialOwnerInput {
                scope: ResourceScope::Organization,
                owner_id: None,
                usage: CredentialUsage::Service,
            },
            name: Some("ORG_KEY".into()),
            label: "Org key".into(),
            mcp: None,
            source: None,
            binding: Some(SecretBinding {
                url: "https://mail.example".into(),
                auth: SecretAuth::Bearer,
            }),
        },
    )
    .await
    .unwrap();
    management::set_value(&ctx, id, "org-secret-sentinel".into(), None, 1)
        .await
        .unwrap();
    let inv = invoke(&db, new_session(&db).await, None).await;
    assert!(
        service::execute(
            &ctx,
            inv,
            "resolve",
            json!({"usage":"service","names":["ORG_KEY"]})
        )
        .await
        .is_err()
    );
    let invalid = sqlx::query(
        r#"
            INSERT INTO resource_access(org_id,kind,scope,owner_agent_id,usage)
            VALUES($1,'credential_binding','organization',$2,'service')
            "#,
    )
    .bind(DEFAULT_ORG_ID)
    .bind(Uuid::now_v7())
    .execute(db.pool())
    .await;
    assert!(invalid.is_err());
}

async fn service_agent(
    db: &StorageBackend,
) -> (
    everruns_contracts::typed_id::AgentId,
    everruns_contracts::typed_id::VirtualUserId,
) {
    use everruns_contracts::typed_id::{AgentId, VirtualUserId};
    let agent = AgentId::from_uuid(db.create_test_agent(DEFAULT_ORG_ID, Uuid::now_v7()).await);
    let user = VirtualUserId::new();
    sqlx::query("INSERT INTO virtual_users(id,org_id,name,usage) VALUES($1,$2,'Credential service','service')")
        .bind(user)
        .bind(DEFAULT_ORG_ID)
        .execute(db.pool()).await.unwrap();
    sqlx::query("UPDATE agents SET virtual_user_id=$2 WHERE id=$1")
        .bind(agent)
        .bind(user)
        .execute(db.pool())
        .await
        .unwrap();
    (agent, user)
}
async fn service_invocation(
    db: &StorageBackend,
    session: SessionId,
    agent: everruns_contracts::typed_id::AgentId,
) -> CredentialInvocation {
    let input = Uuid::now_v7();
    db.record_runtime_invocation(
        DEFAULT_ORG_ID,
        session,
        input,
        None,
        None,
        Some(agent.uuid()),
    )
    .await
    .unwrap();
    CredentialInvocation {
        session_id: session,
        input_message_id: input,
    }
}
#[tokio::test]
async fn explicit_org_grants_follow_the_responder_and_revocation_blocks_next_use() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = context(db.clone());
    let (agent, _) = service_agent(&db).await;
    let (other, _) = service_agent(&db).await;
    let session = new_session(&db).await;
    let inv = service_invocation(&db, session, agent).await;
    let id = management::create(
        &ctx,
        management::CreateScopedCredential {
            owner: management::CredentialOwnerInput {
                scope: ResourceScope::Organization,
                owner_id: None,
                usage: CredentialUsage::Service,
            },
            name: Some("ORG_TOKEN".into()),
            label: "Org token".into(),
            binding: Some(SecretBinding {
                url: "https://api.example/v1".into(),
                auth: SecretAuth::Bearer,
            }),
            mcp: None,
            source: None,
        },
    )
    .await
    .unwrap();
    management::set_value(&ctx, id, "org-token-sentinel".into(), None, 1)
        .await
        .unwrap();
    let read = json!({"usage":"service","names":["ORG_TOKEN"]});
    assert!(
        service::execute(&ctx, inv, "resolve", read.clone())
            .await
            .is_err()
    );
    let mut agent_ctx = ctx.clone();
    agent_ctx.acting_for_session = Some(session);
    assert!(
        management::grant(
            &agent_ctx,
            id,
            management::GrantCredential {
                consumer_scope: ResourceScope::Agent,
                consumer_id: Some(agent.to_string()),
                usage: CredentialUsage::Service,
                expires_at: None,
                constraints: None
            }
        )
        .await
        .is_err()
    );
    let grant = management::grant(
        &ctx,
        id,
        management::GrantCredential {
            consumer_scope: ResourceScope::Agent,
            consumer_id: Some(agent.to_string()),
            usage: CredentialUsage::Service,
            expires_at: None,
            constraints: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        service::execute(&ctx, inv, "resolve", read.clone())
            .await
            .unwrap()[0]["value"],
        "org-token-sentinel"
    );
    let guest = service_invocation(&db, session, other).await;
    assert!(
        service::execute(&ctx, guest, "resolve", read.clone())
            .await
            .is_err()
    );
    management::revoke_grant(&ctx, id, grant).await.unwrap();
    assert!(service::execute(&ctx, inv, "resolve", read).await.is_err());
    let audit: i64 =
        sqlx::query_scalar("SELECT count(*) FROM resource_access_audit WHERE resource_id=$1")
            .bind(id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(audit >= 4);
}
#[tokio::test]
async fn virtual_user_mcp_bindings_inject_only_for_their_identity_and_exact_endpoint() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = context(db.clone());
    let (agent, user) = service_agent(&db).await;
    let (other, _) = service_agent(&db).await;
    let session = new_session(&db).await;
    let id = management::create(
        &ctx,
        management::CreateScopedCredential {
            owner: management::CredentialOwnerInput {
                scope: ResourceScope::VirtualUser,
                owner_id: Some(user.to_string()),
                usage: CredentialUsage::Service,
            },
            name: None,
            label: "MCP token".into(),
            binding: None,
            mcp: Some(management::McpCredentialTarget {
                server_name: "controlled".into(),
                url: "https://api.example/mcp/".into(),
                tool_name: "send".into(),
                parameter_name: "api-key".into(),
            }),
            source: None,
        },
    )
    .await
    .unwrap();
    management::set_value(&ctx, id, "mcp-sentinel".into(), None, 1)
        .await
        .unwrap();
    let inv = service_invocation(&db, session, agent).await;
    let resolve = |inv, url: &'static str| {
        crate::domains::agents::credentials::resolve_runtime_secret_bindings(
            &db,
            ctx.encryption.as_deref(),
            DEFAULT_ORG_ID,
            Some(inv),
            everruns_core::McpServerActsAs::Service,
            "controlled",
            url,
        )
    };
    let value = resolve(inv, "https://api.example/mcp/").await.unwrap();
    assert_eq!(value["send"][0].value.as_deref(), Some("mcp-sentinel"));
    let mismatch = resolve(inv, "https://api.example/mcp").await.unwrap();
    assert!(mismatch["send"][0].value.is_none());
    let guest = service_invocation(&db, session, other).await;
    assert!(
        resolve(guest, "https://api.example/mcp/")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        management::set_value(&ctx, id, "stale-value".into(), None, 1)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn credential_consent_requires_the_proved_person_even_for_an_org_owner() {
    let db = Arc::new(StorageBackend::test_database());
    let runtime = context(db.clone());
    let session = new_session(&db).await;
    let alice_id = db.create_test_user(Uuid::now_v7()).await;
    db.add_organization_member(DEFAULT_ORG_ID, alice_id, "owner")
        .await
        .unwrap();
    let alice = db
        .default_virtual_user(DEFAULT_ORG_ID, alice_id)
        .await
        .unwrap();
    let bob_id = db.create_test_user(Uuid::now_v7()).await;
    db.add_organization_member(DEFAULT_ORG_ID, bob_id, "owner")
        .await
        .unwrap();
    let input = Uuid::now_v7();
    db.record_runtime_invocation(
        DEFAULT_ORG_ID,
        session,
        input,
        Some(alice.id),
        Some(alice_id),
        None,
    )
    .await
    .unwrap();
    let inv = CredentialInvocation {
        session_id: session,
        input_message_id: input,
    };
    let prep = service::execute(
        &runtime,
        inv,
        "prepare",
        json!(plan(ResourceScope::VirtualUser)),
    )
    .await
    .unwrap();
    let ticket: Uuid = prep["ticket"].as_str().unwrap().parse().unwrap();
    let mut alice_ctx = runtime.clone();
    alice_ctx.caller.user_id = Some(alice_id);
    alice_ctx.caller.is_internal = false;
    alice_ctx.caller.is_platform_user = false;
    let mut bob_ctx = alice_ctx.clone();
    bob_ctx.caller.user_id = Some(bob_id);
    assert!(
        service::authorize_capture_answer(&bob_ctx, session, ticket)
            .await
            .is_err()
    );
    assert!(
        service::authorize_capture_answer(&runtime, session, ticket)
            .await
            .is_err()
    );
    assert!(
        service::authorize_capture_answer(&alice_ctx, session, ticket)
            .await
            .is_ok()
    );
    alice_ctx.caller.read_only = true;
    assert!(
        service::authorize_capture_answer(&alice_ctx, session, ticket)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn private_access_depends_on_workspace_audience_and_not_matching_uuid_values() {
    let db = Arc::new(StorageBackend::test_database());
    let ctx = context(db.clone());
    let workspace = Uuid::now_v7();
    sqlx::query("INSERT INTO workspaces(id,org_id,public_id,name) VALUES($1,$2,$3,'Private credential fixture')").bind(workspace).bind(DEFAULT_ORG_ID).bind(format!("wsp_{}",workspace.simple())).execute(db.pool()).await.unwrap();
    let private = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: PrincipalId::from_seed(1),
            workspace_id: Some(workspace),
            ..Default::default()
        })
        .await
        .unwrap()
        .id;
    let inv = invoke(&db, private, Some("alice")).await;
    assert!(
        crate::domains::resource_access::actor(&ctx, inv, CredentialUsage::User)
            .await
            .is_ok()
    );
    db.create_session(CreateSessionRow {
        org_id: DEFAULT_ORG_ID,
        owner_principal_id: PrincipalId::from_seed(1),
        workspace_id: Some(workspace),
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(
        crate::domains::resource_access::actor(&ctx, inv, CredentialUsage::User)
            .await
            .is_err()
    );
    let channel = new_session(&db).await;
    sqlx::query("UPDATE sessions SET source='slack' WHERE id=$1")
        .bind(channel)
        .execute(db.pool())
        .await
        .unwrap();
    let external = invoke(&db, channel, Some("alice")).await;
    assert!(
        crate::domains::resource_access::actor(&ctx, external, CredentialUsage::User)
            .await
            .is_err()
    );
}
