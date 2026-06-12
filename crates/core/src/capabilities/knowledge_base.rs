//! Knowledge Base capability (EVE-423)
//!
//! Binds an agent or harness to one or more org-scoped Knowledge Bases. See
//! `specs/knowledge-bases.md` for the durable design.
//!
//! This module registers the capability and validates the structural shape of
//! its config (`bases[]` entries: `kb_`-prefixed Knowledge Base IDs;
//! `kinds[]` entries from a fixed enum). Domain-level cross-validation
//! (cross-org references, archived/deleted KBs) and the runtime
//! `search_knowledge` tool ship in follow-up vertical slices on top of this
//! foundation.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Capability, CapabilityLocalization, CapabilityStatus, RiskLevel};

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

pub struct KnowledgeBaseCapability;

impl Capability for KnowledgeBaseCapability {
    fn id(&self) -> &str {
        KNOWLEDGE_BASE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Knowledge Base"
    }

    fn description(&self) -> &str {
        "Bind an agent to curated org Knowledge Bases. Used to ground answers in \
         human-edited table docs, business rules, validated SQL templates, and \
         runbooks. The runtime `search_knowledge` tool ships in a follow-up PR; \
         see `specs/knowledge-bases.md`."
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
}
