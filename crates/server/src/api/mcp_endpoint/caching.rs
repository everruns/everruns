// MCP 2026-07-28 cacheable results (spec: server/utilities/caching).
//
// Spec: specs/mcp.md
//
// `2026-07-28` requires every complete result to carry `resultType: "complete"`,
// and requires the listing/reading operations to carry caching hints so clients
// and gateways can avoid re-fetching:
//
//   ttlMs      how long the client MAY consider the result fresh (>= 0)
//   cacheScope "public"  — no user-specific data; ANY caller may reuse it
//              "private" — reusable only within the same authorization context
//
// Decision: hints are emitted only when the negotiated protocol is
// `2026-07-28`. They are additive and 2025-era clients would ignore them, but
// gating keeps each era's responses exactly what that era specifies.
//
// Decision: `resources/read` is `private`. Its payloads are org-scoped
// (capabilities, harnesses, models, agents), so a shared cache keyed only by
// URI would leak one org's data to another. `tools/list` and `resources/list`
// are `public` because both are static per protocol version — see the
// per-constant notes below.

use serde_json::{Value, json};

/// Cache scope, per the 2026-07-28 caching spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheScope {
    /// No user-specific data; any caller or shared proxy may reuse the result.
    Public,
    /// Reusable only within the same authorization context.
    Private,
}

impl CacheScope {
    fn as_str(self) -> &'static str {
        match self {
            CacheScope::Public => "public",
            CacheScope::Private => "private",
        }
    }
}

/// TTL for `tools/list`. The catalog is a compile-time constant keyed only by
/// protocol version, so the sole reason to re-fetch is a deploy. Five minutes
/// bounds how long a client can hold a stale catalog across one.
pub const TOOLS_LIST_TTL_MS: u64 = 300_000;

/// TTL for `resources/list`. Same reasoning as `tools/list` — the resource
/// catalog is a static four-entry list.
pub const RESOURCES_LIST_TTL_MS: u64 = 300_000;

/// TTL for `resources/read`. These payloads are live org data (agents,
/// capabilities, harnesses, models) that changes through ordinary product use,
/// so the window is short enough that a client re-reads within a working
/// session rather than caching a snapshot for minutes.
pub const RESOURCES_READ_TTL_MS: u64 = 30_000;

/// Whether caching hints and `resultType` apply to the negotiated protocol.
fn supports_result_metadata(protocol_version: &str) -> bool {
    protocol_version == super::MCP_PROTOCOL_VERSION_LATEST
}

/// Mark a result complete and attach caching hints (2026-07-28 only).
///
/// No-ops for 2025-era protocol versions and for non-object results.
pub fn mark_cacheable(result: &mut Value, protocol_version: &str, ttl_ms: u64, scope: CacheScope) {
    if !supports_result_metadata(protocol_version) {
        return;
    }
    let Some(object) = result.as_object_mut() else {
        return;
    };
    object.insert("resultType".to_string(), json!("complete"));
    object.insert("ttlMs".to_string(), json!(ttl_ms));
    object.insert("cacheScope".to_string(), json!(scope.as_str()));
}

/// Mark a non-cacheable result complete (2026-07-28 only).
///
/// Used for `tools/call`, which carries `resultType` but no caching hints.
/// Leaves an existing `resultType` alone so the Tasks extension's
/// `resultType: "task"` is not overwritten.
pub fn mark_complete(result: &mut Value, protocol_version: &str) {
    if !supports_result_metadata(protocol_version) {
        return;
    }
    let Some(object) = result.as_object_mut() else {
        return;
    };
    if !object.contains_key("resultType") {
        object.insert("resultType".to_string(), json!("complete"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::mcp_endpoint::{MCP_PROTOCOL_VERSION_2025_06, MCP_PROTOCOL_VERSION_LATEST};

    #[test]
    fn attaches_hints_for_2026_only() {
        let mut result = json!({ "tools": [] });
        mark_cacheable(
            &mut result,
            MCP_PROTOCOL_VERSION_LATEST,
            TOOLS_LIST_TTL_MS,
            CacheScope::Public,
        );
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["ttlMs"], TOOLS_LIST_TTL_MS);
        assert_eq!(result["cacheScope"], "public");

        let mut older = json!({ "tools": [] });
        mark_cacheable(
            &mut older,
            MCP_PROTOCOL_VERSION_2025_06,
            TOOLS_LIST_TTL_MS,
            CacheScope::Public,
        );
        assert!(older.get("ttlMs").is_none());
        assert!(older.get("resultType").is_none());
        assert!(older.get("cacheScope").is_none());
    }

    #[test]
    fn private_scope_serializes_as_private() {
        let mut result = json!({ "contents": [] });
        mark_cacheable(
            &mut result,
            MCP_PROTOCOL_VERSION_LATEST,
            RESOURCES_READ_TTL_MS,
            CacheScope::Private,
        );
        assert_eq!(result["cacheScope"], "private");
        assert_eq!(result["ttlMs"], RESOURCES_READ_TTL_MS);
    }

    #[test]
    fn mark_complete_never_overwrites_a_task_result() {
        let mut task = json!({ "resultType": "task", "taskId": "abc" });
        mark_complete(&mut task, MCP_PROTOCOL_VERSION_LATEST);
        assert_eq!(task["resultType"], "task");

        let mut plain = json!({ "content": [] });
        mark_complete(&mut plain, MCP_PROTOCOL_VERSION_LATEST);
        assert_eq!(plain["resultType"], "complete");
        // Non-cacheable results carry no caching hints.
        assert!(plain.get("ttlMs").is_none());
    }
}
