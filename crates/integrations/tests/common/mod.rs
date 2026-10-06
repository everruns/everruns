#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]
//! Session-service doubles and helpers shared by the Modal test binaries.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::runtime::capabilities::Capability;
use everruns_contracts::runtime::leased_resource::{
    LeasedResource, LeasedResourceStatus, UpsertLeasedResource,
};
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::runtime::{
    connection_services::UserConnectionResolver, session_services::KeyInfo,
    session_services::LeasedResourceStore, session_services::SecretInfo,
    session_services::SessionStorageStore, tool_context::ToolContext,
};
use everruns_contracts::typed_id::{LeasedResourceId, SessionId};
use everruns_integrations::modal::ModalCapability;
use serde_json::Value;
use tokio::sync::Mutex;

// ============================================================================
// Mock session services
// ============================================================================

#[derive(Default)]
pub struct MockStorageStore {
    pub secrets: Mutex<HashMap<String, String>>,
}

#[async_trait]
impl SessionStorageStore for MockStorageStore {
    async fn set_value(&self, _: SessionId, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    async fn get_value(&self, _: SessionId, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
    async fn delete_value(&self, _: SessionId, _: &str) -> Result<bool> {
        Ok(false)
    }
    async fn list_keys(&self, _: SessionId) -> Result<Vec<KeyInfo>> {
        Ok(vec![])
    }
    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()> {
        self.secrets
            .lock()
            .await
            .insert(format!("{session_id}:{name}"), value.to_string());
        Ok(())
    }
    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
        Ok(self
            .secrets
            .lock()
            .await
            .get(&format!("{session_id}:{name}"))
            .cloned())
    }
    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
        Ok(self
            .secrets
            .lock()
            .await
            .remove(&format!("{session_id}:{name}"))
            .is_some())
    }
    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>> {
        let prefix = format!("{session_id}:");
        Ok(self
            .secrets
            .lock()
            .await
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .map(|name| SecretInfo {
                name: name.to_string(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .collect())
    }
}

#[derive(Default)]
pub struct MockLeasedResourceStore {
    pub resources: Mutex<Vec<LeasedResource>>,
}

#[async_trait]
impl LeasedResourceStore for MockLeasedResourceStore {
    async fn upsert_resource(&self, input: UpsertLeasedResource) -> Result<LeasedResource> {
        let now = chrono::Utc::now();
        let resource = LeasedResource {
            id: LeasedResourceId::new(),
            session_id: Some(input.session_id),
            provider: input.provider,
            resource_type: input.resource_type,
            external_id: input.external_id,
            display_name: input.display_name,
            status: LeasedResourceStatus::Active,
            owner_user_id: input.owner_user_id,
            lease_duration_seconds: input.lease_duration_seconds,
            last_touched_at: now,
            lease_expires_at: now
                + chrono::TimeDelta::seconds(i64::from(input.lease_duration_seconds)),
            cleanup_started_at: None,
            cleanup_completed_at: None,
            cleanup_attempts: 0,
            last_cleanup_error: None,
            metadata: input.metadata,
            created_at: now,
            updated_at: now,
        };
        let mut resources = self.resources.lock().await;
        resources.retain(|r| {
            !(r.session_id == resource.session_id
                && r.provider == resource.provider
                && r.external_id == resource.external_id)
        });
        resources.push(resource.clone());
        Ok(resource)
    }

    async fn release_resource(
        &self,
        session_id: SessionId,
        provider: &str,
        resource_type: &str,
        external_id: &str,
    ) -> Result<Option<LeasedResource>> {
        let mut resources = self.resources.lock().await;
        let found = resources.iter_mut().find(|r| {
            r.session_id == Some(session_id)
                && r.provider == provider
                && r.resource_type == resource_type
                && r.external_id == external_id
        });
        Ok(found.map(|r| {
            r.status = LeasedResourceStatus::Released;
            r.clone()
        }))
    }

    async fn list_resources(&self, session_id: SessionId) -> Result<Vec<LeasedResource>> {
        Ok(self
            .resources
            .lock()
            .await
            .iter()
            .filter(|r| r.session_id == Some(session_id))
            .cloned()
            .collect())
    }
}

pub struct MockConnectionResolver(pub Option<String>);

#[async_trait]
impl UserConnectionResolver for MockConnectionResolver {
    async fn get_connection_token(&self, _: SessionId, provider: &str) -> Result<Option<String>> {
        assert_eq!(provider, "modal");
        Ok(self.0.clone())
    }
}

/// Resolves several providers' connection tokens.
pub struct MapConnectionResolver(pub HashMap<String, String>);

#[async_trait]
impl UserConnectionResolver for MapConnectionResolver {
    async fn get_connection_token(&self, _: SessionId, provider: &str) -> Result<Option<String>> {
        Ok(self.0.get(provider).cloned())
    }
}

/// A tool context whose connections are `(provider, token)` pairs.
pub fn context_with_connections(
    connections: &[(&str, &str)],
) -> (ToolContext, Arc<MockLeasedResourceStore>) {
    let leases = Arc::new(MockLeasedResourceStore::default());
    let map = connections
        .iter()
        .map(|(p, t)| ((*p).to_string(), (*t).to_string()))
        .collect();
    let context =
        ToolContext::with_storage_store(SessionId::new(), Arc::new(MockStorageStore::default()))
            .with_leased_resource_store(leases.clone())
            .with_connection_resolver(Arc::new(MapConnectionResolver(map)));
    (context, leases)
}

/// A tool context with fresh in-memory stores and the given connection token.
pub fn context(
    token: Option<&str>,
) -> (
    ToolContext,
    Arc<MockStorageStore>,
    Arc<MockLeasedResourceStore>,
) {
    let storage = Arc::new(MockStorageStore::default());
    let leases = Arc::new(MockLeasedResourceStore::default());
    let context = ToolContext::with_storage_store(SessionId::new(), storage.clone())
        .with_leased_resource_store(leases.clone())
        .with_connection_resolver(Arc::new(MockConnectionResolver(token.map(str::to_string))));
    (context, storage, leases)
}

pub fn tool(name: &str) -> Box<dyn Tool> {
    ModalCapability
        .tools()
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("tool {name} not found"))
}

pub async fn call(context: &ToolContext, name: &str, args: Value) -> ToolExecutionResult {
    tool(name).execute_with_context(args, context).await
}

pub fn ok(result: ToolExecutionResult) -> Value {
    match result {
        ToolExecutionResult::Success(value) => value,
        other => panic!("expected success, got {other:?}"),
    }
}

pub fn tool_error(result: ToolExecutionResult) -> String {
    match result {
        ToolExecutionResult::ToolError(message) => message,
        other => panic!("expected tool error, got {other:?}"),
    }
}
