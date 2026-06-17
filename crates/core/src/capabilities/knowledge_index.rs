//! Knowledge Index capability.
//!
//! Binds an agent or harness to one or more org-scoped Knowledge Indexes —
//! source-backed, embedded collections searched semantically with citations.
//! See `specs/knowledge-indexes.md` for the durable design.
//!
//! This module registers the capability and validates the structural shape of
//! its config (`indexes[]` entries: `kidx_`-prefixed Knowledge Index IDs;
//! optional `top_k` bound). Domain-level cross-validation (cross-org
//! references, archived/deleted indexes) and the runtime `search_index` tool
//! ship in follow-up vertical slices on top of this foundation.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Capability, CapabilityLocalization, CapabilityStatus, RiskLevel};

/// Stable string id for the knowledge index capability.
pub const KNOWLEDGE_INDEX_CAPABILITY_ID: &str = "knowledge_index";

/// Maximum value accepted for the `top_k` result cap.
const MAX_TOP_K: u32 = 50;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KnowledgeIndexConfig {
    /// Knowledge Index IDs the agent can search. Empty/null = no indexes bound.
    #[serde(default)]
    pub indexes: Vec<String>,
    /// Optional default cap on returned results (1..=50). None = tool default.
    #[serde(default)]
    pub top_k: Option<u32>,
}

pub fn validate_knowledge_index_config(cfg: &KnowledgeIndexConfig) -> Result<(), String> {
    for index in &cfg.indexes {
        if !is_valid_index_id(index) {
            return Err(format!(
                "knowledge_index indexes[*] must be a kidx_<32-hex> id, got '{index}'"
            ));
        }
    }
    let mut seen = std::collections::HashSet::new();
    for index in &cfg.indexes {
        if !seen.insert(index) {
            return Err(format!(
                "knowledge_index indexes[*] contains duplicate '{index}'"
            ));
        }
    }
    if let Some(top_k) = cfg.top_k
        && !(1..=MAX_TOP_K).contains(&top_k)
    {
        return Err(format!(
            "knowledge_index top_k must be between 1 and {MAX_TOP_K}, got {top_k}"
        ));
    }
    Ok(())
}

fn is_valid_index_id(s: &str) -> bool {
    // "kidx_" (5) + 32 lowercase hex chars.
    s.len() == 37
        && s.starts_with("kidx_")
        && s[5..]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

pub struct KnowledgeIndexCapability;

impl Capability for KnowledgeIndexCapability {
    fn id(&self) -> &str {
        KNOWLEDGE_INDEX_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Knowledge Index"
    }

    fn description(&self) -> &str {
        "Bind an agent to org Knowledge Indexes — source-backed collections \
         (e.g. a GitHub repository) that are synced, chunked, and embedded for \
         semantic search with citations. The runtime `search_index` tool ships \
         in a follow-up PR; see `specs/knowledge-indexes.md`."
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
        // Retrieval surfaces untrusted external content into the agent context
        // (prompt-injection vector), unlike org-curated Knowledge Bases.
        RiskLevel::Medium
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "indexes": {
                    "type": "array",
                    "title": "Knowledge Indexes",
                    "description": "Knowledge Index IDs the agent can search.",
                    "items": {
                        "type": "string",
                        "title": "Knowledge Index ID",
                        "description": "Knowledge Index ID (kidx_<32-hex>).",
                        "pattern": "^kidx_[0-9a-f]{32}$"
                    }
                },
                "top_k": {
                    "type": "integer",
                    "title": "Result limit",
                    "description": "Optional default cap on returned results.",
                    "minimum": 1,
                    "maximum": 50
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
                    "Selects which Knowledge Indexes the agent can search and an optional \
                     default result limit.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Індекс знань"),
                description: Some(
                    "Прив'язує агента до Індексів знань організації — колекцій із зовнішніх \
                     джерел (наприклад, репозиторій GitHub), які синхронізуються, розбиваються \
                     на фрагменти та векторизуються для семантичного пошуку з посиланнями.",
                ),
                config_description: Some(
                    "Визначає, у яких Індексах знань агент може шукати, та необов'язкову \
                     типову межу кількості результатів.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "indexes": {
                            "title": "Індекси знань",
                            "description": "Ідентифікатори Індексів знань, у яких агент може шукати.",
                            "items": {
                                "title": "Ідентифікатор Індексу знань",
                                "description": "Ідентифікатор Індексу знань (kidx_<32-hex>)."
                            }
                        },
                        "top_k": {
                            "title": "Межа результатів",
                            "description": "Необов'язкова типова межа кількості повернених результатів."
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
        let typed: KnowledgeIndexConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("invalid knowledge_index config: {e}"))?;
        validate_knowledge_index_config(&typed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_ID: &str = "kidx_00000000000000000000000000000001";

    #[test]
    fn id_and_name() {
        let cap = KnowledgeIndexCapability;
        assert_eq!(cap.id(), "knowledge_index");
        assert_eq!(cap.name(), "Knowledge Index");
    }

    #[test]
    fn validate_accepts_empty_config() {
        let cap = KnowledgeIndexCapability;
        assert!(cap.validate_config(&json!({})).is_ok());
        assert!(cap.validate_config(&json!({ "indexes": [] })).is_ok());
        assert!(cap.validate_config(&Value::Null).is_ok());
    }

    #[test]
    fn validate_accepts_well_formed_config() {
        let cap = KnowledgeIndexCapability;
        let cfg = json!({ "indexes": [VALID_ID], "top_k": 10 });
        assert!(cap.validate_config(&cfg).is_ok());
    }

    #[test]
    fn validate_rejects_malformed_index_id() {
        let cap = KnowledgeIndexCapability;
        let cfg = json!({ "indexes": ["kb_00000000000000000000000000000001"] });
        let err = cap.validate_config(&cfg).unwrap_err();
        assert!(err.contains("kidx_"));
    }

    #[test]
    fn validate_rejects_duplicate_indexes() {
        let cap = KnowledgeIndexCapability;
        let cfg = json!({ "indexes": [VALID_ID, VALID_ID] });
        let err = cap.validate_config(&cfg).unwrap_err();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn validate_rejects_out_of_range_top_k() {
        let cap = KnowledgeIndexCapability;
        assert!(cap.validate_config(&json!({ "top_k": 0 })).is_err());
        assert!(cap.validate_config(&json!({ "top_k": 51 })).is_err());
        assert!(cap.validate_config(&json!({ "top_k": 25 })).is_ok());
    }

    #[test]
    fn uk_localization_present() {
        let cap = KnowledgeIndexCapability;
        assert_eq!(cap.localized_name(Some("uk-UA")), "Індекс знань");
        assert!(cap.describe_schema(Some("uk")).is_some());
        assert!(cap.describe_schema(None).is_some());
    }
}
