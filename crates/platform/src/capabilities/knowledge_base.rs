//! Knowledge Base capability (EVE-423)
//!
//! Binds an agent or harness to one or more org-scoped Knowledge Bases. See
//! `knowledge/runtime-resources/knowledge-bases.md` for the durable design.
//!
//! This module registers the capability, validates the structural shape of its
//! config (`bases[]`: `kb_`-prefixed Knowledge Base IDs; `kinds[]` from a fixed
//! enum), and provides the agent-facing `search_knowledge` tool. Org scoping of
//! the bound bases is enforced by the `KnowledgeStore` implementation at search
//! time (cross-org ids are silently skipped). See knowledge/runtime-resources/okf-adoption.md.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Capability, CapabilityLocalization, CapabilityStatus, RiskLevel};
use crate::knowledge_store::KnowledgeStoreExt;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool, ToolExecutionResult};

/// Stable string id for the knowledge base capability.
pub const KNOWLEDGE_BASE_CAPABILITY_ID: &str = "knowledge_base";

/// Allowed entry kinds. Must stay in sync with the SQL CHECK constraint in
/// `crates/server/migrations/032_knowledge_bases.sql` and with
/// `crates/server/src/domains/knowledge_bases::ENTRY_KINDS`.
const ENTRY_KINDS: &[&str] = &["note", "table", "business", "query", "runbook"];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KnowledgeBaseConfig {
    /// Knowledge Base IDs the agent can search. Empty/null = no bases bound.
    #[serde(default)]
    pub bases: Vec<String>,
    /// Optional default kind filter applied when the agent does not pass
    /// `kind` explicitly. Empty/null = no filter.
    #[serde(default)]
    pub kinds: Vec<String>,
}

pub fn validate_knowledge_base_config(cfg: &KnowledgeBaseConfig) -> Result<(), String> {
    for base in &cfg.bases {
        if !is_valid_kb_id(base) {
            return Err(format!(
                "knowledge_base bases[*] must be a kb_<32-hex> id, got '{base}'"
            ));
        }
    }
    let mut seen = std::collections::HashSet::new();
    for base in &cfg.bases {
        if !seen.insert(base) {
            return Err(format!(
                "knowledge_base bases[*] contains duplicate '{base}'"
            ));
        }
    }
    for kind in &cfg.kinds {
        if !ENTRY_KINDS.contains(&kind.as_str()) {
            return Err(format!(
                "knowledge_base kinds[*] must be one of {:?}, got '{kind}'",
                ENTRY_KINDS
            ));
        }
    }
    Ok(())
}

fn is_valid_kb_id(s: &str) -> bool {
    s.len() == 35
        && s.starts_with("kb_")
        && s[3..]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// Agent-facing tool that searches the bound Knowledge Bases. Holds the
/// capability's config (which `bases` to search, default `kinds`), populated at
/// collection time via `tools_with_config`. See knowledge/runtime-resources/okf-adoption.md.
pub struct SearchKnowledgeTool {
    config: KnowledgeBaseConfig,
}

impl SearchKnowledgeTool {
    pub fn new(config: KnowledgeBaseConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for SearchKnowledgeTool {
    fn narrate(
        &self,
        tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(everruns_core::tool_narration::narrate_search_knowledge(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "search_knowledge"
    }

    fn description(&self) -> &str {
        "Search curated organization Knowledge Bases (table docs, business rules, \
         validated query templates, runbooks) by keyword. Consult this before \
         answering data questions and cite results by their kbe_ id."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Keyword search across entry title and body" },
                "kind": {
                    "type": "string",
                    "enum": ["note", "table", "business", "query", "runbook"],
                    "description": "Optional filter by entry kind"
                },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Optional tag filter" },
                "limit": { "type": "integer", "minimum": 1, "maximum": 25, "default": 10 }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("search_knowledge requires execution context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let query = match arguments.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.trim().is_empty() => q.trim().to_string(),
            _ => return ToolExecutionResult::tool_error("Missing required parameter: query"),
        };

        // No bound bases → empty result set (valid per spec).
        if self.config.bases.is_empty() {
            return ToolExecutionResult::success(json!({ "count": 0, "results": [] }));
        }

        let Some(store) = context.extension::<KnowledgeStoreExt>() else {
            return ToolExecutionResult::tool_error(
                "Knowledge search is not available in this execution context",
            );
        };
        let Some(org_id) = context.org_id else {
            return ToolExecutionResult::tool_error(
                "Knowledge search requires an organization context",
            );
        };

        // Explicit kind wins; otherwise fall back to the configured default filter.
        // The store filters by a single kind. Apply a configured default only
        // when exactly one kind is configured; with multiple, search across all
        // kinds rather than silently honoring just the first.
        let kind = arguments
            .get("kind")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| match self.config.kinds.as_slice() {
                [single] => Some(single.clone()),
                _ => None,
            });
        let tags: Vec<String> = arguments
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_lowercase()))
                    .collect()
            })
            .unwrap_or_default();
        let limit = arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| (v as usize).clamp(1, 25))
            .unwrap_or(10);

        match store
            .0
            .search_knowledge(
                org_id,
                &self.config.bases,
                &query,
                kind.as_deref(),
                &tags,
                limit,
            )
            .await
        {
            Ok(hits) => ToolExecutionResult::success(json!({
                "count": hits.len(),
                "results": hits,
            })),
            Err(e) => {
                ToolExecutionResult::internal_error_msg(format!("knowledge search failed: {e}"))
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn deferrable_policy(&self) -> everruns_provider::tool_types::DeferrablePolicy {
        // "Consult-first" tool: keep its full schema directly callable so the
        // model invokes it rather than routing through tool-search. See
        // knowledge/execution/tool-search.md (never-defer) and knowledge/runtime-resources/okf-adoption.md.
        everruns_provider::tool_types::DeferrablePolicy::Never
    }
}

pub struct KnowledgeBaseCapability;

impl Capability for KnowledgeBaseCapability {
    fn id(&self) -> &str {
        KNOWLEDGE_BASE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Knowledge Base"
    }

    fn description(&self) -> &str {
        "Bind an agent to curated org Knowledge Bases and give it the \
         `search_knowledge` tool to ground answers in human-edited table docs, \
         business rules, validated SQL templates, and runbooks. \
         See `knowledge/runtime-resources/knowledge-bases.md`."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("library")
    }

    fn category(&self) -> Option<&str> {
        Some("Knowledge")
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["knowledge"]
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Low
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "You can search curated organization knowledge with the `search_knowledge` tool \
             (table docs, business rules, validated queries, runbooks). Consult it before \
             answering data questions, and cite the entries you use by their kbe_ id.",
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&Value::Null)
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        let cfg: KnowledgeBaseConfig = serde_json::from_value(config.clone()).unwrap_or_default();
        vec![Box::new(SearchKnowledgeTool::new(cfg))]
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "bases": {
                    "type": "array",
                    "title": "Knowledge Bases",
                    "description": "Knowledge Base IDs the agent can search.",
                    "items": {
                        "type": "string",
                        "title": "Knowledge Base ID",
                        "description": "Knowledge Base ID (kb_<32-hex>).",
                        "pattern": "^kb_[0-9a-f]{32}$"
                    }
                },
                "kinds": {
                    "type": "array",
                    "title": "Default kind filter",
                    "description": "Optional kind filter applied when the agent does not pass `kind`.",
                    "items": {
                        "type": "string",
                        "title": "Entry kind",
                        "description": "Knowledge Base entry kind to include.",
                        // Values must stay in sync with ENTRY_KINDS (enforced by test).
                        "oneOf": [
                            { "const": "note", "title": "Note" },
                            { "const": "table", "title": "Table doc" },
                            { "const": "business", "title": "Business rule" },
                            { "const": "query", "title": "Query template" },
                            { "const": "runbook", "title": "Runbook" }
                        ]
                    }
                }
            }
        }))
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Selects which Knowledge Bases the agent can search and an optional \
                     default entry-kind filter.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("База знань"),
                description: Some(
                    "Прив'язує агента до курованих Баз знань організації, щоб відповіді \
                     спиралися на редаговані людьми описи таблиць, бізнес-правила, \
                     перевірені SQL-шаблони та runbook-и.",
                ),
                config_description: Some(
                    "Визначає, у яких Базах знань агент може шукати, та необов'язковий \
                     типовий фільтр за видом записів.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "bases": {
                            "title": "Бази знань",
                            "description": "Ідентифікатори Баз знань, у яких агент може шукати.",
                            "items": {
                                "title": "Ідентифікатор Бази знань",
                                "description": "Ідентифікатор Бази знань (kb_<32-hex>)."
                            }
                        },
                        "kinds": {
                            "title": "Типовий фільтр виду",
                            "description": "Необов'язковий фільтр за видом записів, що застосовується, коли агент не передає kind явно.",
                            "items": {
                                "title": "Вид запису",
                                "description": "Вид записів Бази знань, який потрібно включити.",
                                "enum_labels": {
                                    "note": "Нотатка",
                                    "table": "Опис таблиці",
                                    "business": "Бізнес-правило",
                                    "query": "Шаблон запиту",
                                    "runbook": "Runbook"
                                }
                            }
                        }
                    }
                })),
            },
        ]
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        let typed: KnowledgeBaseConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("invalid knowledge_base config: {e}"))?;
        validate_knowledge_base_config(&typed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_name() {
        let cap = KnowledgeBaseCapability;
        assert_eq!(cap.id(), "knowledge_base");
        assert_eq!(cap.name(), "Knowledge Base");
    }

    #[test]
    fn validate_accepts_empty_config() {
        let cap = KnowledgeBaseCapability;
        assert!(cap.validate_config(&json!({})).is_ok());
        assert!(cap.validate_config(&json!({ "bases": [] })).is_ok());
        assert!(cap.validate_config(&Value::Null).is_ok());
    }

    #[test]
    fn validate_accepts_well_formed_bases_and_kinds() {
        let cap = KnowledgeBaseCapability;
        let cfg = json!({
            "bases": ["kb_00000000000000000000000000000001"],
            "kinds": ["table", "business"]
        });
        assert!(cap.validate_config(&cfg).is_ok());
    }

    #[test]
    fn validate_rejects_malformed_kb_id() {
        let cap = KnowledgeBaseCapability;
        let cfg = json!({ "bases": ["mem_00000000000000000000000000000001"] });
        let err = cap.validate_config(&cfg).unwrap_err();
        assert!(err.contains("kb_"));
    }

    #[test]
    fn validate_rejects_duplicate_bases() {
        let cap = KnowledgeBaseCapability;
        let cfg = json!({
            "bases": [
                "kb_00000000000000000000000000000001",
                "kb_00000000000000000000000000000001"
            ]
        });
        let err = cap.validate_config(&cfg).unwrap_err();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn validate_rejects_unknown_kind() {
        let cap = KnowledgeBaseCapability;
        let cfg = json!({ "kinds": ["nope"] });
        let err = cap.validate_config(&cfg).unwrap_err();
        assert!(err.contains("kinds"));
    }

    #[test]
    fn uk_localization_and_schema_one_of_match_validation() {
        let cap = KnowledgeBaseCapability;
        assert_eq!(cap.localized_name(Some("uk-UA")), "База знань");
        assert!(
            cap.localized_description(Some("uk-UA"))
                .contains("Баз знань")
        );
        assert!(cap.describe_schema(Some("uk")).is_some());
        assert!(cap.describe_schema(None).is_some());

        // The kinds oneOf consts must be exactly the values validate_config accepts.
        let schema = cap.config_schema().expect("config schema");
        let consts: Vec<&str> = schema["properties"]["kinds"]["items"]["oneOf"]
            .as_array()
            .expect("oneOf")
            .iter()
            .map(|v| v["const"].as_str().expect("const"))
            .collect();
        assert_eq!(consts, ENTRY_KINDS);
        for kind in consts {
            assert!(cap.validate_config(&json!({ "kinds": [kind] })).is_ok());
        }
    }

    // --- search_knowledge tool ---

    use crate::knowledge_store::{KnowledgeSearchHit, KnowledgeStore, KnowledgeStoreExt};
    use everruns_core::tool_context::ToolContext;
    use everruns_provider::typed_id::{DEFAULT_ORG_ID, SessionId};
    use std::sync::Arc;

    struct MockKnowledgeStore {
        hits: Vec<KnowledgeSearchHit>,
    }

    #[async_trait]
    impl KnowledgeStore for MockKnowledgeStore {
        async fn search_knowledge(
            &self,
            _org_id: everruns_provider::typed_id::OrgId,
            kb_public_ids: &[String],
            _query: &str,
            _kind: Option<&str>,
            _tags: &[String],
            _limit: usize,
        ) -> everruns_provider::error::Result<Vec<KnowledgeSearchHit>> {
            if kb_public_ids.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(self.hits.clone())
            }
        }
    }

    fn hit() -> KnowledgeSearchHit {
        KnowledgeSearchHit {
            id: "kbe_00000000000000000000000000000001".into(),
            kb_id: "kb_00000000000000000000000000000001".into(),
            title: "Orders".into(),
            kind: "table".into(),
            tags: vec!["sales".into()],
            snippet: "One row per order.".into(),
            resource: None,
        }
    }

    #[tokio::test]
    async fn search_tool_returns_results_from_store() {
        let tool = SearchKnowledgeTool::new(KnowledgeBaseConfig {
            bases: vec!["kb_00000000000000000000000000000001".into()],
            kinds: vec![],
        });
        let mut ctx = ToolContext::new(SessionId::new());
        ctx.extensions
            .insert(Arc::new(KnowledgeStoreExt(Arc::new(MockKnowledgeStore {
                hits: vec![hit()],
            }))));
        ctx.org_id = Some(DEFAULT_ORG_ID);

        let result = tool
            .execute_with_context(json!({ "query": "orders" }), &ctx)
            .await;
        match result {
            ToolExecutionResult::Success(v) => {
                assert_eq!(v["count"], 1);
                assert_eq!(
                    v["results"][0]["id"],
                    "kbe_00000000000000000000000000000001"
                );
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn search_tool_with_no_bases_returns_empty() {
        let tool = SearchKnowledgeTool::new(KnowledgeBaseConfig::default());
        let mut ctx = ToolContext::new(SessionId::new());
        ctx.extensions
            .insert(Arc::new(KnowledgeStoreExt(Arc::new(MockKnowledgeStore {
                hits: vec![hit()],
            }))));
        ctx.org_id = Some(DEFAULT_ORG_ID);

        let result = tool
            .execute_with_context(json!({ "query": "orders" }), &ctx)
            .await;
        match result {
            ToolExecutionResult::Success(v) => assert_eq!(v["count"], 0),
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn search_tool_requires_query() {
        let tool = SearchKnowledgeTool::new(KnowledgeBaseConfig {
            bases: vec!["kb_00000000000000000000000000000001".into()],
            kinds: vec![],
        });
        let ctx = ToolContext::new(SessionId::new());
        let result = tool.execute_with_context(json!({}), &ctx).await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }
}
