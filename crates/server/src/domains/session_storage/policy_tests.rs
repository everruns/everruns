//! Session storage reads honor a custom permission resolver.
//!
//! Listing returns plaintext key/value entries and secret names. Org ownership
//! is not a substitute for `SESSION_VIEW`: a same-org caller the resolver
//! denies must learn neither. Writes stay on `SESSION_MANAGE`.

use std::sync::Arc;

use everruns_core::{
    Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, DefaultPermissionResolver, OrgRole, Permission,
    PermissionResolver,
};
use everruns_provider::typed_id::SessionId;
use uuid::Uuid;

use crate::domains::common::{Command, CommandErrorKind, Ctx, dispatch};
use crate::domains::sessions::{SESSION_MANAGE, SESSION_VIEW};
use crate::storage::StorageBackend;
use crate::storage::models::{CreateSessionRow, UpsertSessionKeyValue, UpsertSessionSecret};

use super::{BatchSetSessionSecrets, DeleteSessionSecret, ListSessionSecrets, ListSessionStorage};

const PLAINTEXT: &str = "payroll-plaintext-9f3a";
const USER_KEY: &str = "payroll";
const SECRET_NAME: &str = "NIGHTLY_PAYROLL_TOKEN";
const INTERNAL_KV_KEY: &str = "tool_approval/forged-decision";
const INTERNAL_SECRET: &str = "session_sandbox";

struct DenySessionAccess;

impl PermissionResolver for DenySessionAccess {
    fn has_permission(&self, _caller: &Caller, permission: &Permission) -> bool {
        permission != &Permission::OrgSessionsManage
    }

    fn caller_permissions(&self, caller: &Caller) -> Vec<Permission> {
        Permission::ALL
            .iter()
            .copied()
            .filter(|permission| self.has_permission(caller, permission))
            .collect()
    }
}

fn owner() -> Caller {
    Caller {
        org_id: DEFAULT_ORG_ID,
        org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
        user_id: Some(Uuid::nil()),
        role: OrgRole::Owner,
        is_platform_user: false,
        is_internal: false,
    }
}

fn ctx(db: Arc<StorageBackend>, resolver: Arc<dyn PermissionResolver>) -> Ctx {
    Ctx::minimal(owner(), db, None, resolver)
}

async fn seed() -> (Arc<StorageBackend>, SessionId) {
    let db = Arc::new(StorageBackend::in_memory());
    let session = db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            title: Some("storage policy".to_string()),
            ..Default::default()
        })
        .await
        .expect("seed session");
    db.upsert_session_key_value(UpsertSessionKeyValue {
        session_id: session.id,
        key: USER_KEY.to_string(),
        value: PLAINTEXT.to_string(),
    })
    .await
    .expect("seed user key");
    db.upsert_session_key_value(UpsertSessionKeyValue {
        session_id: session.id,
        key: INTERNAL_KV_KEY.to_string(),
        value: "hidden-approval".to_string(),
    })
    .await
    .expect("seed internal key");
    db.upsert_session_secret(UpsertSessionSecret {
        session_id: session.id,
        name: SECRET_NAME.to_string(),
        value_encrypted: b"ciphertext".to_vec(),
    })
    .await
    .expect("seed user secret");
    db.upsert_session_secret(UpsertSessionSecret {
        session_id: session.id,
        name: INTERNAL_SECRET.to_string(),
        value_encrypted: b"sandbox".to_vec(),
    })
    .await
    .expect("seed internal secret");
    (db, session.id)
}

fn assert_forbidden(err: crate::domains::common::CommandError, route: &str) {
    assert!(
        matches!(err.kind, CommandErrorKind::Forbidden(_)),
        "{route}: expected Forbidden, got {err:?}"
    );
    let message = err.message();
    assert!(
        !message.contains(PLAINTEXT) && !message.contains(SECRET_NAME),
        "{route} leaked storage contents: {message}"
    );
}

#[tokio::test]
async fn denied_caller_cannot_read_storage_or_secret_names() {
    let (db, session_id) = seed().await;
    let denied = ctx(db, Arc::new(DenySessionAccess));
    let session_id = session_id.to_string();

    let list_keys = ListSessionStorage {
        session_id: session_id.clone(),
    }
    .run(&denied)
    .await
    .expect_err("list_session_storage must require SESSION_VIEW");
    assert_forbidden(list_keys, "list keys");

    let list_secrets = ListSessionSecrets {
        session_id: session_id.clone(),
    }
    .run(&denied)
    .await
    .expect_err("list_session_secrets must require SESSION_VIEW");
    assert_forbidden(list_secrets, "list secrets");

    let dispatched_keys = dispatch(
        "list_session_storage",
        serde_json::json!({ "session_id": session_id }),
        &denied,
    )
    .await
    .expect_err("MCP/gRPC dispatch must require SESSION_VIEW");
    assert_forbidden(dispatched_keys, "dispatch keys");

    let dispatched_secrets = dispatch(
        "list_session_secrets",
        serde_json::json!({ "session_id": session_id }),
        &denied,
    )
    .await
    .expect_err("MCP/gRPC dispatch must require SESSION_VIEW");
    assert_forbidden(dispatched_secrets, "dispatch secrets");
}

#[tokio::test]
async fn authorized_caller_reads_user_entries_and_hides_internal_ones() {
    let (db, session_id) = seed().await;
    let allowed = ctx(db, Arc::new(DefaultPermissionResolver));
    let session_id = session_id.to_string();

    let keys = ListSessionStorage {
        session_id: session_id.clone(),
    }
    .run(&allowed)
    .await
    .expect("authorized caller lists storage");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].key, USER_KEY);
    assert_eq!(keys[0].value, PLAINTEXT);

    let secrets = ListSessionSecrets {
        session_id: session_id.clone(),
    }
    .run(&allowed)
    .await
    .expect("authorized caller lists secret names");
    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets[0].name, SECRET_NAME);

    let dispatched = dispatch(
        "list_session_storage",
        serde_json::json!({ "session_id": &session_id }),
        &allowed,
    )
    .await
    .expect("dispatch lists storage");
    assert!(dispatched.contains(PLAINTEXT));
    assert!(!dispatched.contains(INTERNAL_KV_KEY));
}

#[tokio::test]
async fn denied_caller_cannot_mutate_storage() {
    let (db, session_id) = seed().await;
    let denied = ctx(db.clone(), Arc::new(DenySessionAccess));
    let allowed = ctx(db, Arc::new(DefaultPermissionResolver));
    let session_id = session_id.to_string();

    let deleted = DeleteSessionSecret {
        session_id: session_id.clone(),
        name: SECRET_NAME.to_string(),
    }
    .run(&denied)
    .await
    .expect_err("delete stays on SESSION_MANAGE");
    assert_forbidden(deleted, "delete secret");

    let written = BatchSetSessionSecrets {
        session_id: session_id.clone(),
        secrets: [("EXTRA_TOKEN".to_string(), "value".to_string())]
            .into_iter()
            .collect(),
    }
    .run(&denied)
    .await
    .expect_err("batch set stays on SESSION_MANAGE");
    assert_forbidden(written, "batch set");

    let secrets = ListSessionSecrets { session_id }
        .run(&allowed)
        .await
        .expect("authorized list after refused writes");
    assert_eq!(secrets.len(), 1);
    assert_eq!(secrets[0].name, SECRET_NAME);
}

#[test]
fn storage_commands_declare_the_session_policies() {
    assert_eq!(
        ListSessionStorage::policy().map(|policy| policy.id),
        Some(SESSION_VIEW.id)
    );
    assert_eq!(
        ListSessionSecrets::policy().map(|policy| policy.id),
        Some(SESSION_VIEW.id)
    );
    assert_eq!(
        BatchSetSessionSecrets::policy().map(|policy| policy.id),
        Some(SESSION_MANAGE.id)
    );
    assert_eq!(
        DeleteSessionSecret::policy().map(|policy| policy.id),
        Some(SESSION_MANAGE.id)
    );
}
