//! The org settings a data plane's tools read as they run (data plane v2
//! parity, Phase A): web search defaults, accounts and their keys, the tool
//! approval policy, the egress grants.
//!
//! These answers are known before any call, so instead of answering them
//! at call time (host calls), the control plane pushes them with the org's
//! config: each setting is an `org_setting` connection whose settings are
//! the value, and each search account's key is a secret sealed with the
//! org's data key. A shard's tools read them from the org's config, so they
//! keep working while the control plane is down, and a standalone shard's
//! are simply absent.
//!
//! Decisions:
//! - Synced whole, from the control plane's tables, whenever one of them
//!   changes (org settings, the egress allowlist and grants, a search
//!   account added, renamed, updated or removed) and when the org is put on
//!   a data plane. An unchanged setting writes nothing; a key is sealed
//!   again only when it changed, and an account the org no longer has
//!   loses its key.
//! - A sync never fails the change that triggered it ([`sync_logged`]),
//!   and is never lost either: a trigger on the settings' tables counts
//!   every change in the same transaction (`dp_orgs.settings_rev`), and
//!   the dispatcher syncs every org whose count is ahead of its last sync
//!   ([`sync_owed`]). Revoking open egress therefore reaches the shards
//!   even when the sync right after the change failed.
//! - One sync of an org at a time, across servers (an advisory lock,
//!   [`settle`]): each reads the change count and the tables under it, so
//!   an older read never publishes after a newer one, and is never
//!   recorded as covering a change it did not see.
//! - The session owner's own approval settings are a person's, not the
//!   org's: they stay a host call (`tool_approval_policy`), and without an
//!   answer the turn runs with none, which only asks more.
//! - THREAT[TM-TOOL-041]: only keys of search accounts (providers with a
//!   search driver) are pushed, so nothing else an org connection holds
//!   reaches the data plane this way.

use std::collections::HashSet;
use std::sync::Arc;

use everruns_contracts::dataplane::{
    self as dp, EGRESS_SETTING, ORG_SETTING, TOOL_APPROVAL_SETTING, WEB_SEARCH_SETTING,
    web_search_key_secret,
};
use serde_json::{Value, json};

use super::{ControlPlane, put_connection, put_secret, remove};
use crate::storage::StorageBackend;

/// The prefix of the search account key secrets.
const KEY_PREFIX: &str = "web-search-key:";

/// The lock that orders one org's syncs, keyed by the org.
const SYNC_LOCK: &str = "dp-org-settings-sync";
/// How long a sync waits for the one running before it.
const SYNC_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// Orgs synced per dispatcher pass.
const OWED_BATCH: i64 = 64;

/// Sync every org whose settings changed since their last sync.
pub async fn sync_owed(cp: &ControlPlane) -> anyhow::Result<usize> {
    let behind = cp.db.dp_orgs_settings_behind(OWED_BATCH).await?;
    let n = behind.len();
    for (org_id, _) in behind {
        if let Err(error) = settle(cp, org_id).await {
            tracing::warn!(org_id, %error, "the data plane's org settings were not synced");
        }
    }
    Ok(n)
}

/// Bring `org_id`'s pushed settings in line with its tables and record the
/// change count it covered. A v1 org is left alone.
pub async fn sync(cp: &ControlPlane, org_id: i64) -> anyhow::Result<()> {
    settle(cp, org_id).await
}

/// Sync `org_id` from its tables and record the change count that covered,
/// one sync of the org at a time across every server. Syncs overlap (a
/// change's own sync on one server, the dispatcher on another); unordered,
/// one that read the tables before a change could publish after that
/// change's own sync, putting back what it took away (open egress after a
/// revocation) while the count says it is synced. Under the lock each sync
/// reads the count, then the tables, then publishes, so the one that read
/// last publishes last.
async fn settle(cp: &ControlPlane, org_id: i64) -> anyhow::Result<()> {
    let _one_at_a_time = cp
        .db
        .advisory_xact_lock_polling(SYNC_LOCK, &org_id.to_string(), SYNC_LOCK_WAIT)
        .await?;
    // Read before the tables: a change made while this sync runs bumps the
    // count past `rev`, so it is synced again.
    let Some(rev) = cp.db.dp_org_settings_rev(org_id).await? else {
        return Ok(());
    };
    sync_tables(cp, org_id).await?;
    cp.db.dp_settings_synced(org_id, rev).await
}

async fn sync_tables(cp: &ControlPlane, org_id: i64) -> anyhow::Result<()> {
    if cp.db.dp_org(org_id).await?.is_none() {
        return Ok(());
    }
    let web_search =
        crate::domains::organizations::web_search::web_search_settings(&cp.db, org_id).await?;
    setting(
        cp,
        org_id,
        WEB_SEARCH_SETTING,
        serde_json::to_value(&web_search)?,
    )
    .await?;
    let approval = cp.db.org_tool_approval_policy(org_id).await?;
    setting(
        cp,
        org_id,
        TOOL_APPROVAL_SETTING,
        serde_json::to_value(&approval)?,
    )
    .await?;
    // What the egress boundary enforces: patterns only while granted, as
    // `worker_get_org_egress_allowlist` answers.
    let egress = cp.db.org_egress_allowlist(org_id).await?;
    let patterns = if egress.granted {
        egress.patterns
    } else {
        Vec::new()
    };
    setting(
        cp,
        org_id,
        EGRESS_SETTING,
        json!({ "patterns": patterns, "open": egress.open }),
    )
    .await?;

    let mut live = HashSet::new();
    for account in &web_search.accounts {
        let Some(row) = cp
            .db
            .get_organization_connection(org_id, account.id)
            .await?
        else {
            continue;
        };
        let Some(encrypted) = row.access_token_encrypted else {
            continue;
        };
        let key = cp.encryption()?.decrypt_to_string(&encrypted)?;
        let id = web_search_key_secret(account.id);
        if super::providers::sealed_key(cp, org_id, &id)
            .await?
            .as_deref()
            != Some(key.as_bytes())
        {
            put_secret(cp, org_id, &id, key.as_bytes()).await?;
        }
        live.insert(id);
    }
    for item in cp.db.dp_items_live(org_id).await? {
        if item.kind == "secret"
            && item.item_id.starts_with(KEY_PREFIX)
            && !live.contains(&item.item_id)
        {
            remove(&cp.db, org_id, "secret", &item.item_id).await?;
        }
    }
    Ok(())
}

/// Put setting `id` as `value`, unless it already is.
async fn setting(cp: &ControlPlane, org_id: i64, id: &str, value: Value) -> anyhow::Result<()> {
    let connection = dp::Connection {
        connection_id: id.to_owned(),
        kind: ORG_SETTING.to_owned(),
        settings: value,
        token_secret_id: String::new(),
        refresh_owner: dp::RefreshOwner::ControlPlane,
    };
    let current = cp.db.dp_item(org_id, "connection", id).await?;
    if current.and_then(|item| item.body)
        != Some(dp::ConfigChange::Connection(connection.clone()).to_value())
    {
        put_connection(&cp.db, org_id, connection).await?;
    }
    Ok(())
}

/// [`sync`] after a change on `db`, logged rather than failed: a settings
/// change in the console never fails because the data plane's copy did not
/// update.
pub async fn sync_logged(db: &Arc<StorageBackend>, org_id: i64) {
    let cp = ControlPlane::from_env(db.clone());
    if let Err(error) = sync(&cp, org_id).await {
        tracing::warn!(org_id, %error, "the data plane's org settings were not synced");
    }
}
