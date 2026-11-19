//! A v2 session's storage (data plane v2, step 11.6).
//!
//! Decisions:
//! - The values and secrets live on the session's cell, where its agent's
//!   tools read them. The Storage API keeps its one code path, policy and
//!   validation here for both planes; for a v2 session it reads them from
//!   the cell (the `storage` read) and changes them with the `storage`
//!   session command.
//! - THREAT[TM-TENANT]: a person's secret is sealed here with the org's
//!   data key, bound to the session and its name, before it leaves the
//!   control plane; the cell refuses anything else and never returns a
//!   secret's value.
//! - A cell keeps no timestamps for a key or secret: their `created_at` and
//!   `updated_at` are empty and the console shows none.
//! - Bound secrets (a secret the platform puts on requests to one URL) are
//!   refused on a v2 session for now: the cell has no egress binding yet.

use everruns_contracts::session_commands;
use std::collections::BTreeMap;

use everruns_contracts::typed_id::SessionId;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::domains::common::*;
use crate::domains::data_plane::session_plane::SessionPlane;
use crate::domains::data_plane::{ControlPlane, org_public};

/// A v2 session's storage as its cell lists it.
#[derive(Debug, Default, Deserialize)]
pub struct Listed {
    #[serde(default)]
    pub values: BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: Vec<String>,
}

/// One of an org's sessions on its data plane, for its storage.
pub struct PlaneStorage {
    plane: SessionPlane,
    cp: ControlPlane,
    org_id: i64,
}

impl PlaneStorage {
    /// `session`'s storage when it runs on a data plane.
    pub async fn of(ctx: &Ctx, session: SessionId) -> Result<Option<Self>, CommandError> {
        Self::on(&ctx.db, ctx.org_id(), session)
            .await
            .map_err(CommandError::internal)
    }

    /// [`PlaneStorage::of`] for `org_id`.
    pub async fn on(
        db: &std::sync::Arc<crate::storage::StorageBackend>,
        org_id: i64,
        session: SessionId,
    ) -> anyhow::Result<Option<Self>> {
        let cp = ControlPlane::from_env(db.clone());
        let plane = SessionPlane::with(&cp, org_id, session).await?;
        Ok(plane.map(|plane| Self { plane, cp, org_id }))
    }

    /// The session itself on its data plane.
    pub fn plane(&self) -> &SessionPlane {
        &self.plane
    }

    /// The session's values and secret names.
    pub async fn listed(&self) -> Result<Listed, CommandError> {
        let value = self
            .plane
            .read(&session_commands::STORAGE_READ)
            .await
            .map_err(CommandError::internal)?;
        serde_json::from_value(value).map_err(|e| CommandError::internal(e.into()))
    }

    /// One value, if set.
    pub async fn value(&self, key: &str) -> Result<Option<String>, CommandError> {
        Ok(self.listed().await?.values.remove(key))
    }

    pub async fn set_value(&self, key: &str, value: &str) -> Result<(), CommandError> {
        self.change(json!({ "kind": "set_value", "key": key, "value": value }))
            .await
            .map(|_| ())
    }

    pub async fn delete_value(&self, key: &str) -> Result<bool, CommandError> {
        self.change(json!({ "kind": "delete_value", "key": key }))
            .await
            .map(|v| v.as_bool().unwrap_or(false))
    }

    /// The org's data key and its id.
    async fn data_key(
        &self,
    ) -> Result<(everruns_contracts::dataplane_seal::Key, String), CommandError> {
        let org = self
            .cp
            .db
            .dp_org(self.org_id)
            .await
            .map_err(CommandError::internal)?
            .ok_or_else(|| {
                CommandError::internal(anyhow::anyhow!("the org is not on a data plane"))
            })?;
        let key = self.cp.data_key(&org).map_err(CommandError::internal)?;
        Ok((key, org.data_key_id))
    }

    /// The session's secrets, opened: what a fork copies, and the
    /// sandbox's state (`domains/session_sandbox/plane.rs`). Only the
    /// control plane holds the org's data key; the cell hands them over
    /// sealed.
    pub async fn secrets(&self) -> Result<BTreeMap<String, String>, CommandError> {
        #[derive(Deserialize)]
        struct Stored {
            #[serde(default)]
            secrets: BTreeMap<String, String>,
        }
        let read = self
            .plane
            .read(&session_commands::SECRETS)
            .await
            .map_err(CommandError::internal)?;
        let stored: Stored =
            serde_json::from_value(read).map_err(|e| CommandError::internal(e.into()))?;
        if stored.secrets.is_empty() {
            return Ok(BTreeMap::new());
        }
        let (key, key_id) = self.data_key().await?;
        let org = org_public(self.org_id);
        let session = self.plane.session.to_string();
        stored
            .secrets
            .into_iter()
            .map(|(name, sealed)| {
                everruns_contracts::dataplane_seal::open_session_secret(
                    &key, &key_id, &org, &session, &name, &sealed,
                )
                .map(|plain| (name, plain))
                .map_err(|e| CommandError::internal(anyhow::anyhow!(e.to_string())))
            })
            .collect()
    }

    /// Seal `value` for this session and `name`, and store it.
    pub async fn set_secret(&self, name: &str, value: &str) -> Result<(), CommandError> {
        let (key, key_id) = self.data_key().await?;
        let sealed = everruns_contracts::dataplane_seal::seal_session_secret(
            &key,
            &key_id,
            &org_public(self.org_id),
            &self.plane.session.to_string(),
            name,
            value,
        )
        .map_err(|e| CommandError::internal(anyhow::anyhow!(e.to_string())))?;
        self.change(json!({ "kind": "set_secret", "name": name, "sealed": sealed }))
            .await
            .map(|_| ())
    }

    pub async fn delete_secret(&self, name: &str) -> Result<bool, CommandError> {
        self.change(json!({ "kind": "delete_secret", "name": name }))
            .await
            .map(|v| v.as_bool().unwrap_or(false))
    }

    async fn change(&self, op: Value) -> Result<Value, CommandError> {
        self.plane
            .command(&session_commands::STORAGE, &op)
            .await
            .map_err(CommandError::internal)
    }
}
