//! Shared lexical retrieval for native discovery and command adapters.
//!
//! The index contains metadata, never execution authority. Callers supply the
//! current scope on every search. Source context is scored once per authorized
//! server or capability, independently of how many operations it owns.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::tool_types::ToolDefinition;
use serde::Serialize;
use serde_json::Value;

const EXACT_NAME_BONUS: f64 = 10_000.0;
const FIELD_WEIGHTS: [f64; 3] = [4.0, 1.0, 1.0];

fn normalized_tool_name(name: &str) -> String {
    name.trim()
        .trim_matches(|character: char| {
            !character.is_alphanumeric() && character != '_' && character != '-'
        })
        .to_lowercase()
        .replace('-', "_")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
}

#[derive(Debug, Clone)]
struct Document {
    name: String,
    lengths: [usize; 3],
    source: Option<String>,
}

/// The owner context used to refine a search without revealing arbitrary tools.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum ToolSearchSourceKind {
    McpServer,
    Capability,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Ord, PartialOrd)]
pub struct ToolSearchSource {
    pub kind: ToolSearchSourceKind,
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone)]
struct Source {
    info: ToolSearchSource,
    terms: HashMap<String, usize>,
}

/// One ranked canonical tool identity. Resolve it against the current registry
/// before returning a schema or executing a call.
#[derive(Debug, Clone)]
pub struct ToolSearchHit {
    pub name: String,
    pub score: f64,
    pub tool_match: bool,
    pub server_match: bool,
    /// Present only when the query matches this tool's source context.
    pub source: Option<ToolSearchSource>,
}

/// An immutable index for one registry revision. No schemas are duplicated in
/// this structure. A registry mutation invalidates its cached index.
#[derive(Debug, Clone, Default)]
pub struct ToolSearchIndex {
    documents: Vec<Document>,
    sources: BTreeMap<String, Source>,
    postings: HashMap<String, Vec<(usize, [usize; 3])>>,
    exact_names: HashMap<String, Vec<usize>>,
}

fn tokens(text: &str) -> Vec<String> {
    let mut normalized = String::new();
    let mut previous_lowercase = false;
    for character in text.chars() {
        if character.is_uppercase() && previous_lowercase {
            normalized.push(' ');
        }
        if character.is_alphanumeric() {
            normalized.extend(character.to_lowercase());
        } else {
            normalized.push(' ');
        }
        previous_lowercase = character.is_lowercase() || character.is_numeric();
    }
    normalized.split_whitespace().map(str::to_owned).collect()
}

fn add_terms(terms: &mut HashMap<String, usize>, text: &str, weight: usize) {
    for term in tokens(text) {
        *terms.entry(term).or_default() += weight;
    }
}

fn schema_terms(schema: &Value, terms: &mut HashMap<String, usize>) {
    // Index names and descriptions, not arbitrary enum/example payloads. The
    // traversal budget also bounds work on unusually broad or nested schemas.
    let mut pending = vec![schema];
    for _ in 0..512 {
        let Some(value) = pending.pop() else { break };
        if let Some(object) = value.as_object() {
            if let Some(description) = object.get("description").and_then(Value::as_str) {
                add_terms(
                    terms,
                    &description.chars().take(1024).collect::<String>(),
                    1,
                );
            }
            if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                for (name, child) in properties.iter().take(128) {
                    add_terms(terms, name, 1);
                    pending.push(child);
                }
            }
            for key in ["items", "additionalProperties"] {
                if let Some(child) = object.get(key).filter(|value| value.is_object()) {
                    pending.push(child);
                }
            }
            for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
                if let Some(children) = object.get(key).and_then(Value::as_array) {
                    pending.extend(children.iter().take(128));
                }
            }
            for key in ["$defs", "definitions"] {
                if let Some(children) = object.get(key).and_then(Value::as_object) {
                    pending.extend(children.values().take(128));
                }
            }
        }
    }
}

impl ToolSearchIndex {
    pub fn new(definitions: &[ToolDefinition]) -> Self {
        let mut definitions: Vec<_> = definitions.iter().collect();
        definitions.sort_by(|left, right| left.name().cmp(right.name()));
        let mut index = Self::default();
        for definition in definitions {
            let hints = definition.hints();
            let metadata = hints.metadata.as_ref();
            let context = metadata.and_then(|metadata| metadata.get("mcp_server"));
            let capability = metadata.and_then(|metadata| metadata.get("capability"));
            let capability_id = hints.capability_id.as_deref().or_else(|| {
                capability
                    .and_then(|context| context.get("id"))
                    .and_then(Value::as_str)
            });
            let capability_name = hints
                .capability_name
                .as_deref()
                .or_else(|| {
                    capability
                        .and_then(|context| context.get("name"))
                        .and_then(Value::as_str)
                })
                .or(capability_id);
            let capability_description = hints.capability_description.as_deref().or_else(|| {
                capability
                    .and_then(|context| context.get("description"))
                    .and_then(Value::as_str)
            });
            let parsed = super::mcp_server::parse_mcp_tool_name(definition.name());
            let server = context
                .and_then(|context| context.get("name"))
                .and_then(Value::as_str)
                .or_else(|| parsed.as_ref().map(|(name, _)| name.as_str()));
            // An MCP tool remains a member of its server, even when a
            // capability supplied it. Combine both contexts in one source so
            // it cannot get two source votes or distort source-level IDF.
            let source_info = server
                .map(|server| ToolSearchSource {
                    kind: ToolSearchSourceKind::McpServer,
                    id: server.to_owned(),
                    name: server.chars().take(256).collect(),
                })
                .or_else(|| {
                    capability_id.map(|id| ToolSearchSource {
                        kind: ToolSearchSourceKind::Capability,
                        id: id.to_owned(),
                        name: capability_name.unwrap_or(id).chars().take(256).collect(),
                    })
                });
            let source = source_info.map(|info| {
                let prefix = match info.kind {
                    ToolSearchSourceKind::McpServer => "mcp:",
                    ToolSearchSourceKind::Capability => "capability:",
                };
                let key = format!("{prefix}{}", info.id);
                index.sources.entry(key.clone()).or_insert_with(|| {
                    let mut terms = HashMap::new();
                    add_terms(&mut terms, &info.id, 1);
                    add_terms(&mut terms, &info.name, 1);
                    if let Some(description) = context
                        .and_then(|context| context.get("description"))
                        .and_then(Value::as_str)
                    {
                        add_terms(
                            &mut terms,
                            &description.chars().take(4096).collect::<String>(),
                            1,
                        );
                    }
                    if let Some(id) = capability_id {
                        add_terms(&mut terms, id, 1);
                    }
                    if let Some(name) = capability_name {
                        add_terms(&mut terms, &name.chars().take(256).collect::<String>(), 1);
                    }
                    if let Some(description) = capability_description {
                        add_terms(
                            &mut terms,
                            &description.chars().take(4096).collect::<String>(),
                            1,
                        );
                    }
                    Source { info, terms }
                });
                key
            });
            // Keep operation names, descriptions and schemas as independent
            // fields. Schema verbosity and shared invocation-envelope terms
            // must not dilute an operation's own name or description.
            let mut fields: [HashMap<String, usize>; 3] = Default::default();
            let operation_name = parsed
                .as_ref()
                .map(|(_, name)| name.as_str())
                .or_else(|| {
                    // Only the declared owner's exact namespace is context.
                    // Match MCP's operation aliases without stripping arbitrary
                    // leading words from ordinary, unowned tool names.
                    let id = capability_id?;
                    definition
                        .name()
                        .strip_prefix(&format!("{id}_"))
                        .or_else(|| {
                            definition
                                .name()
                                .strip_prefix(&format!("{}_", id.replace('-', "_")))
                        })
                        .filter(|operation| !operation.is_empty())
                })
                .unwrap_or(definition.name());
            add_terms(&mut fields[0], operation_name, 1);
            add_terms(&mut fields[1], definition.description(), 1);
            schema_terms(definition.full_parameters(), &mut fields[2]);
            let lengths = std::array::from_fn(|field| fields[field].values().sum());
            let mut terms: HashMap<String, [usize; 3]> = HashMap::new();
            for (field, field_terms) in fields.into_iter().enumerate() {
                for (term, frequency) in field_terms {
                    terms.entry(term).or_default()[field] = frequency;
                }
            }
            let position = index.documents.len();
            index
                .exact_names
                .entry(normalized_tool_name(definition.name()))
                .or_default()
                .push(position);
            if operation_name != definition.name() {
                index
                    .exact_names
                    .entry(normalized_tool_name(operation_name))
                    .or_default()
                    .push(position);
            }
            for (term, frequency) in &terms {
                index
                    .postings
                    .entry(term.clone())
                    .or_default()
                    .push((position, *frequency));
            }
            index.documents.push(Document {
                name: definition.name().to_owned(),
                lengths,
                source,
            });
        }
        index
    }

    /// Deduplicated owner contexts with at least one currently authorized tool.
    pub fn sources(&self, allowed: impl Fn(&str) -> bool) -> Vec<ToolSearchSource> {
        self.documents
            .iter()
            .filter(|document| allowed(&document.name))
            .filter_map(|document| document.source.as_ref())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|key| self.sources[key].info.clone())
            .collect()
    }

    /// Rank matching owner contexts once each, independently of tool count.
    /// This keeps bounded refinements from being consumed by sibling tools.
    pub fn search_sources(
        &self,
        query: &str,
        limit: usize,
        allowed: impl Fn(&str) -> bool,
    ) -> Vec<ToolSearchSource> {
        if limit == 0 {
            return Vec::new();
        }
        let sources = self
            .documents
            .iter()
            .filter(|document| allowed(&document.name))
            .filter_map(|document| document.source.as_deref())
            .collect();
        let terms = tokens(query).into_iter().collect();
        let mut matches: Vec<_> = self
            .score_sources(&terms, &sources)
            .into_iter()
            .filter(|(_, score)| *score > 0.0)
            .collect();
        matches.sort_by(|(left, left_score), (right, right_score)| {
            right_score
                .total_cmp(left_score)
                .then_with(|| self.sources[*left].info.cmp(&self.sources[*right].info))
        });
        matches
            .into_iter()
            .take(limit)
            .map(|(key, _)| self.sources[key].info.clone())
            .collect()
    }

    fn score_sources<'a>(
        &self,
        terms: &HashSet<String>,
        sources: &HashSet<&'a str>,
    ) -> HashMap<&'a str, f64> {
        sources
            .iter()
            .map(|source| {
                let score = terms
                    .iter()
                    .filter(|term| self.sources[*source].terms.contains_key(*term))
                    .map(|term| {
                        let frequency = sources
                            .iter()
                            .filter(|source| self.sources[**source].terms.contains_key(term))
                            .count();
                        (1.0 + (sources.len() as f64 - frequency as f64 + 0.5)
                            / (frequency as f64 + 0.5))
                            .ln()
                    })
                    .sum();
                (*source, score)
            })
            .collect()
    }

    /// Search only identities admitted by the caller's current authorization
    /// view. Forbidden documents do not contribute to IDF or source ranking.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        allowed: impl Fn(&str) -> bool,
    ) -> Vec<ToolSearchHit> {
        self.search_filtered(query, limit, allowed, false)
    }

    /// Rank operation matches separately from source-only browse candidates,
    /// so a large matching source cannot crowd real operations out of top-k.
    pub fn search_operations(
        &self,
        query: &str,
        limit: usize,
        allowed: impl Fn(&str) -> bool,
    ) -> Vec<ToolSearchHit> {
        self.search_filtered(query, limit, allowed, true)
    }

    fn search_filtered(
        &self,
        query: &str,
        limit: usize,
        allowed: impl Fn(&str) -> bool,
        operations_only: bool,
    ) -> Vec<ToolSearchHit> {
        let visible: Vec<_> = self
            .documents
            .iter()
            .map(|document| allowed(&document.name))
            .collect();
        let count = visible.iter().filter(|allowed| **allowed).count();
        if count == 0 || limit == 0 {
            return Vec::new();
        }
        let terms: HashSet<_> = tokens(query).into_iter().collect();
        let averages: [f64; 3] = std::array::from_fn(|field| {
            self.documents
                .iter()
                .zip(&visible)
                .filter(|(_, allowed)| **allowed)
                .map(|(document, _)| document.lengths[field])
                .sum::<usize>() as f64
                / count as f64
        });
        let sources: HashSet<_> = self
            .documents
            .iter()
            .zip(&visible)
            .filter(|(_, allowed)| **allowed)
            .filter_map(|(document, _)| document.source.as_deref())
            .collect();
        let source_scores = self.score_sources(&terms, &sources);
        let mut scores = vec![0.0; self.documents.len()];
        for term in &terms {
            let Some(postings) = self.postings.get(term) else {
                continue;
            };
            // IDF is field-local too: a word shared by every schema can still
            // identify a rare operation name. Sum weighted BM25 field scores
            // without cutting partial matches from multi-operation workflows.
            let idfs: [f64; 3] = std::array::from_fn(|field| {
                let frequency = postings
                    .iter()
                    .filter(|(position, frequencies)| visible[*position] && frequencies[field] > 0)
                    .count();
                (1.0 + (count as f64 - frequency as f64 + 0.5) / (frequency as f64 + 0.5)).ln()
            });
            for &(position, frequencies) in
                postings.iter().filter(|(position, _)| visible[*position])
            {
                for field in 0..FIELD_WEIGHTS.len() {
                    let frequency = frequencies[field] as f64;
                    let normalization = 1.5
                        * (0.25
                            + 0.75 * self.documents[position].lengths[field] as f64
                                / averages[field].max(1.0));
                    scores[position] += FIELD_WEIGHTS[field] * idfs[field] * frequency * 2.5
                        / (frequency + normalization);
                }
            }
        }
        let exact = normalized_tool_name(query);
        let exact_names: HashSet<_> = self
            .exact_names
            .get(&exact)
            .into_iter()
            .flatten()
            .filter(|position| visible[**position])
            .map(|position| self.documents[*position].name.as_str())
            .collect();
        let mut hits: Vec<_> = self
            .documents
            .iter()
            .enumerate()
            .filter(|(position, _)| visible[*position])
            .filter_map(|(position, document)| {
                let source_score = document
                    .source
                    .as_deref()
                    .and_then(|source| source_scores.get(source))
                    .copied()
                    .unwrap_or(0.0);
                let exact_match = exact_names.contains(document.name.as_str());
                let tool_match = scores[position] > 0.0 || exact_match;
                let score = scores[position]
                    + source_score
                    + if exact_match { EXACT_NAME_BONUS } else { 0.0 };
                ((score > 0.0 || terms.is_empty()) && (!operations_only || tool_match)).then(|| {
                    ToolSearchHit {
                        name: document.name.clone(),
                        score,
                        tool_match,
                        server_match: source_score > 0.0
                            && document.source.as_ref().is_some_and(|key| {
                                self.sources[key].info.kind == ToolSearchSourceKind::McpServer
                            }),
                        source: document
                            .source
                            .as_ref()
                            .and_then(|key| self.sources.get(key))
                            .filter(|_| source_score > 0.0)
                            .map(|source| source.info.clone()),
                    }
                })
            })
            .collect();
        hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.name.cmp(&right.name))
        });
        // A complete operation name, including its words separated by spaces
        // or hyphens, expresses exact intent. Keep every authorized identity,
        // including the same operation on multiple servers, and discard
        // incidental word hits.
        if !exact_names.is_empty() {
            hits.retain(|hit| exact_names.contains(hit.name.as_str()));
        }
        hits.truncate(limit);
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tools::{Tool, ToolExecutionResult, ToolRegistry};
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Arc;

    fn operation(server: &str, operation: &str, description: &str) -> ToolDefinition {
        let mut definition = ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
            name: format!("mcp_{server}__{operation}"),
            description: description.into(),
            parameters: json!({"type":"object","properties":{}}),
            display_name: None,
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
            full_parameters: None,
        });
        if let ToolDefinition::Builtin(tool) = &mut definition {
            tool.hints.metadata = Some(json!({"mcp_server": {
                "name":server, "description": if server == "atlas" {
                    "Warehouse inventory and fulfillment"
                } else {"Service deployment and monitoring"}
            }}));
        }
        definition
    }

    #[test]
    fn server_context_finds_generic_operations_without_cardinality_votes() {
        let mut definitions = vec![operation("atlas", "get", "Read one item")];
        definitions
            .extend((0..2000).map(|n| operation("cloud", &format!("get_{n:04}"), "Read one item")));
        let index = ToolSearchIndex::new(&definitions);
        let hits = index.search("warehouse fulfillment", 8, |_| true);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "mcp_atlas__get");
        assert!(hits[0].server_match);
        assert!(!hits[0].tool_match);
        // Extra operations on a server do not increase its context score.
        let small = ToolSearchIndex::new(&[
            operation("atlas", "get", "Read one item"),
            operation("cloud", "get", "Read one item"),
        ]);
        assert_eq!(
            hits[0].score,
            small.search("warehouse fulfillment", 8, |_| true)[0].score
        );
    }

    #[test]
    fn current_scope_controls_results_and_ranking_statistics() {
        let authorized = operation("atlas", "get", "Read warehouse stock");
        let index = ToolSearchIndex::new(&[
            authorized.clone(),
            operation("cloud", "get", "Read warehouse stock"),
        ]);
        let hits = index.search("warehouse", 8, |name| name == authorized.name());
        let isolated = ToolSearchIndex::new(&[authorized]).search("warehouse", 8, |_| true);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].score, isolated[0].score);
        assert!(index.search("warehouse", 8, |_| false).is_empty());
    }

    #[test]
    fn broad_rollback_search_preserves_all_four_specific_operations() {
        let mut definitions = vec![
            operation("ops", "rollback_deployment", "Restore one deployment"),
            operation("ops", "rollback_transaction", "Restore one transaction"),
            operation("ops", "rollback_release", "Restore one release"),
            operation("ops", "rollback_migration", "Restore one migration"),
        ];
        for definition in &mut definitions {
            if let ToolDefinition::Builtin(tool) = definition {
                // Keep server metadata out of this operation-domain retrieval case.
                tool.hints.metadata = None;
            }
        }
        definitions.extend((0..2000).map(|index| {
            let mut definition =
                operation("misc", &format!("get_{index:04}"), "A rollback endpoint");
            if let ToolDefinition::Builtin(tool) = &mut definition {
                tool.hints.metadata = None;
            }
            definition
        }));

        let index = ToolSearchIndex::new(&definitions);
        let hits = index.search_operations(
            "rollback deployment transaction release migration",
            8,
            |_| true,
        );
        let names: HashSet<_> = hits.iter().map(|hit| hit.name.as_str()).collect();
        for operation in [
            "mcp_ops__rollback_deployment",
            "mcp_ops__rollback_transaction",
            "mcp_ops__rollback_release",
            "mcp_ops__rollback_migration",
        ] {
            assert!(names.contains(operation), "missing {operation}");
        }
    }

    #[test]
    fn fixture_deployment_query_keeps_the_complete_four_operation_workflow() {
        let fixture_server = json!({
            "name":"telemetry",
            "description":"Geothermal drilling telemetry and field operations records"
        });
        let mut definitions = vec![
            operation(
                "telemetry",
                "list_deployments",
                "Find deployments by service, environment, and status. Use this first to identify the exact deployment before inspection or change.",
            ),
            operation(
                "telemetry",
                "get_deployment",
                "Read one deployment, including current version, previous version, health checks, and rollback eligibility.",
            ),
            operation(
                "telemetry",
                "rollback_deployment",
                "Roll a failed deployment back to its recorded previous version. Requires the exact deployment id and an explicit reason; this mutates deployment state.",
            ),
            operation(
                "telemetry",
                "verify_deployment",
                "Verify deployment health and report the observed version and state.",
            ),
        ];
        for definition in &mut definitions {
            if let ToolDefinition::Builtin(tool) = definition {
                tool.hints.metadata = Some(json!({"mcp_server": fixture_server}));
                tool.parameters = json!({"type":"object","properties":{
                    "deployment_id":{"type":"string","description":"Exact deployment identifier returned by list_deployments."},
                    "service":{"type":"string","description":"Service slug, for example billing-api."},
                    "environment":{"type":"string","description":"Deployment environment.","enum":["dev","staging","prod"]}
                }});
            }
        }
        definitions.extend((0..1996).map(|index| {
            let mut definition = operation(
                "telemetry",
                &format!("inspect_billing_record_{index:04}"),
                "Inspect a billing operation record by its exact key.",
            );
            if let ToolDefinition::Builtin(tool) = &mut definition {
                tool.hints.metadata = Some(json!({"mcp_server": fixture_server}));
            }
            definition
        }));

        let index = ToolSearchIndex::new(&definitions);
        let hits = index.search_operations("deployment staging billing", 8, |_| true);
        let names: HashSet<_> = hits.iter().map(|hit| hit.name.as_str()).collect();
        for operation in [
            "mcp_telemetry__list_deployments",
            "mcp_telemetry__get_deployment",
            "mcp_telemetry__rollback_deployment",
            "mcp_telemetry__verify_deployment",
        ] {
            assert!(names.contains(operation), "missing {operation}");
        }
    }

    #[test]
    fn exact_mcp_identity_discards_incidental_hits_and_unqualified_name_keeps_server_twins() {
        let definitions = [
            operation("telemetry", "get_deployment", "Read one deployment"),
            operation("platform", "get_deployment", "Read one deployment"),
            operation("misc", "get_status", "Get deployment status"),
        ];
        let index = ToolSearchIndex::new(&definitions);

        for query in [
            "mcp_telemetry__get_deployment",
            "mcp_telemetry__get-deployment",
            "mcp_telemetry__get deployment",
        ] {
            let qualified = index.search_operations(query, 8, |_| true);
            assert_eq!(qualified.len(), 1, "{query}");
            assert_eq!(qualified[0].name, "mcp_telemetry__get_deployment");
        }

        for query in [
            "get_deployment",
            "get-deployment",
            "get deployment",
            "get \t deployment",
        ] {
            let unqualified = index.search_operations(query, 8, |_| true);
            let names: HashSet<_> = unqualified.iter().map(|hit| hit.name.as_str()).collect();
            assert_eq!(unqualified.len(), 2, "{query}");
            assert!(names.contains("mcp_telemetry__get_deployment"));
            assert!(names.contains("mcp_platform__get_deployment"));
            let scoped =
                index.search_operations(query, 8, |name| name.starts_with("mcp_telemetry__"));
            assert_eq!(scoped.len(), 1);
            assert_eq!(scoped[0].name, "mcp_telemetry__get_deployment");
        }

        let phrase = index.search_operations("get deployment status", 8, |_| true);
        assert_eq!(
            phrase.len(),
            3,
            "a broader phrase should keep BM25 semantics"
        );
    }

    #[test]
    fn schema_fields_references_and_exact_qualified_names_are_searchable() {
        let mut target = operation("atlas", "get", "Read one item");
        if let ToolDefinition::Builtin(tool) = &mut target {
            tool.parameters = json!({"type":"object","properties":{"trackingId":{"type":"string"}},
                "$defs":{"item":{"description":"Shipment provenance"}}});
        }
        let index = ToolSearchIndex::new(&[target, operation("cloud", "get", "Read one item")]);
        for query in ["tracking", "provenance", "`mcp_atlas__get`"] {
            assert_eq!(index.search(query, 1, |_| true)[0].name, "mcp_atlas__get");
        }
        assert!(index.search("volcanic", 8, |_| true).is_empty());
        assert_eq!(index.search("", 1, |_| true)[0].name, "mcp_atlas__get");
    }

    fn invocation_schema(property_count: usize) -> Value {
        let properties: serde_json::Map<_, _> = (0..property_count)
            .map(|position| {
                (
                    format!("option_{position}"),
                    json!({"type":"string", "description":
                        "Optional configuration details for this entity. ".repeat(20)}),
                )
            })
            .collect();
        json!({"type":"object", "properties":{
            "params":{"type":"object","properties":properties},
            "reason":{"type":"string","description":"Required for agent entity changes."},
            "context_revision":{"type":"integer","description":"Manager context revision read before changing an existing entity."}
        }})
    }

    fn with_schema(mut definition: ToolDefinition, schema: Value) -> ToolDefinition {
        if let ToolDefinition::Builtin(tool) = &mut definition {
            tool.parameters = schema;
        }
        definition
    }

    #[test]
    fn verbose_schemas_do_not_suppress_operation_names_or_descriptions() {
        let mut description_score: Option<f64> = None;
        for property_count in [0, 8, 64, 128] {
            let mut definitions = vec![
                with_schema(
                    operation("platform", "update_agent", "Update an agent."),
                    invocation_schema(property_count),
                ),
                with_schema(
                    operation("platform", "resolve", "Reconcile invoice obligations."),
                    invocation_schema(property_count),
                ),
            ];
            definitions.extend((0..32).map(|position| {
                with_schema(
                    operation(
                        "platform",
                        &format!("update_record_{position}"),
                        "Update a record and reconcile its metadata.",
                    ),
                    invocation_schema(0),
                )
            }));
            let index = ToolSearchIndex::new(&definitions);
            for (query, expected) in [
                ("please update agent", "mcp_platform__update_agent"),
                ("reconcile invoice obligations", "mcp_platform__resolve"),
            ] {
                let hits = index.search_operations(query, 8, |_| true);
                assert_eq!(
                    hits[0].name, expected,
                    "{property_count} properties: {query}"
                );
                if query == "reconcile invoice obligations" {
                    if let Some(original) = description_score {
                        assert!(
                            (hits[0].score - original).abs() < 1e-9,
                            "Unrelated schema text must not change description scores"
                        );
                    } else {
                        description_score = Some(hits[0].score);
                    }
                }
            }
        }
    }

    #[test]
    fn shared_schema_terms_do_not_erase_distinctive_name_terms() {
        let mut definitions = vec![with_schema(
            operation(
                "platform",
                "get_manager_context",
                "Read the notes managers keep about an entity before changing it.",
            ),
            invocation_schema(128),
        )];
        definitions.extend((0..32).map(|position| {
            with_schema(
                operation(
                    "platform",
                    &format!("get_record_{position}"),
                    "Get one record.",
                ),
                invocation_schema(0),
            )
        }));
        let index = ToolSearchIndex::new(&definitions);
        let hits = index.search_operations("get agent manager context", 8, |_| true);
        assert_eq!(hits[0].name, "mcp_platform__get_manager_context");
    }

    struct CatalogTool(ToolDefinition);
    #[async_trait]
    impl Tool for CatalogTool {
        fn name(&self) -> &str {
            self.0.name()
        }
        fn description(&self) -> &str {
            self.0.description()
        }
        fn parameters_schema(&self) -> Value {
            self.0.parameters().clone()
        }
        fn to_definition(&self) -> ToolDefinition {
            self.0.clone()
        }
        async fn execute(&self, arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::success(arguments)
        }
    }

    #[test]
    fn registry_reuses_index_and_invalidates_every_mutation_path() {
        let mut registry = ToolRegistry::new();
        registry.register(CatalogTool(operation("atlas", "get", "Old turquoise")));
        let original = registry.search_index();
        assert!(Arc::ptr_eq(&original, &registry.search_index()));
        let mut fork = registry.clone();
        fork.register_boxed(Box::new(CatalogTool(operation(
            "atlas",
            "get",
            "New shipments",
        ))));
        let replaced = fork.search_index();
        assert!(!Arc::ptr_eq(&original, &replaced));
        assert!(replaced.search("turquoise", 8, |_| true).is_empty());
        assert_eq!(replaced.search("shipments", 8, |_| true).len(), 1);
        assert_eq!(original.search("turquoise", 8, |_| true).len(), 1);
        fork.register_arc(Arc::new(CatalogTool(operation(
            "cloud",
            "deploy",
            "Deploy release",
        ))));
        assert!(!Arc::ptr_eq(&replaced, &fork.search_index()));
        fork.unregister("mcp_atlas__get");
        assert!(
            fork.search_index()
                .search("shipments", 8, |_| true)
                .is_empty()
        );
        fork.clear();
        assert!(fork.search_index().search("", 8, |_| true).is_empty());
    }

    #[test]
    fn ten_thousand_tools_support_repeated_bounded_queries() {
        let definitions: Vec<_> = (0..10_000)
            .map(|n| operation("cloud", &format!("get_{n:05}"), "Read one item"))
            .collect();
        let index = ToolSearchIndex::new(&definitions);
        for n in 0..100 {
            let name = format!("mcp_cloud__get_{:05}", n * 97);
            let hits = index.search(&name, 8, |_| true);
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].name, name);
            assert!(hits[0].tool_match);
        }
    }

    #[test]
    fn server_metadata_change_reindexes_without_a_provider_schema_change() {
        let original = operation("atlas", "get", "Read one item");
        let mut updated = original.clone();
        if let ToolDefinition::Builtin(tool) = &mut updated {
            tool.hints.metadata = Some(json!({"mcp_server":{
                "name":"atlas", "description":"Geothermal telemetry"}}));
        }
        assert!(crate::tool_changes::same_for_model(&original, &updated));
        assert!(
            ToolSearchIndex::new(&[original])
                .search("geothermal", 8, |_| true)
                .is_empty()
        );
        assert!(ToolSearchIndex::new(&[updated]).search("geothermal", 8, |_| true)[0].server_match);
    }

    fn capability_operation(
        name: &str,
        id: &str,
        display: &str,
        description: &str,
    ) -> ToolDefinition {
        let mut definition = operation("unused", "unused", "Read one item");
        if let ToolDefinition::Builtin(tool) = &mut definition {
            tool.name = name.to_owned();
            tool.hints.metadata = None;
            tool.hints.capability_id = Some(id.to_owned());
            tool.hints.capability_name = Some(display.to_owned());
            tool.hints.capability_description = Some(description.to_owned());
        }
        definition
    }

    #[test]
    fn capability_context_discovers_by_id_name_and_description_without_revealing_operations() {
        let definition = capability_operation(
            "peek",
            "internal_stock",
            "Warehouse",
            "Fulfillment logistics",
        );
        let index = ToolSearchIndex::new(&[definition]);
        let expected = ToolSearchSource {
            kind: ToolSearchSourceKind::Capability,
            id: "internal_stock".into(),
            name: "Warehouse".into(),
        };
        for query in ["internal_stock", "warehouse", "logistics"] {
            let hits = index.search(query, 8, |_| true);
            assert_eq!(hits.len(), 1, "{query}");
            assert_eq!(hits[0].source, Some(expected.clone()));
            assert!(!hits[0].tool_match);
            assert!(!hits[0].server_match);
            assert!(index.search_operations(query, 8, |_| true).is_empty());
        }
        assert_eq!(index.sources(|_| true), vec![expected]);
        let operation = index.search_operations("peek", 8, |_| true);
        assert!(operation[0].tool_match);
        assert!(operation[0].source.is_none());
    }

    #[test]
    fn legacy_capability_attribution_and_metadata_context_remain_searchable() {
        let mut definition =
            capability_operation("peek", "internal_stock", "Warehouse", "Logistics");
        if let ToolDefinition::Builtin(tool) = &mut definition {
            tool.hints.capability_description = None;
        }
        let legacy = ToolSearchIndex::new(&[definition.clone()]);
        assert_eq!(legacy.search("warehouse", 8, |_| true).len(), 1);
        assert_eq!(legacy.search("internal_stock", 8, |_| true).len(), 1);
        if let ToolDefinition::Builtin(tool) = &mut definition {
            tool.hints.capability_id = None;
            tool.hints.capability_name = None;
            tool.hints.metadata = Some(json!({"capability": {
                "id": "internal_stock", "name": "Warehouse", "description": "Logistics"
            }}));
        }
        let metadata = ToolSearchIndex::new(&[definition]);
        assert_eq!(metadata.search("logistics", 8, |_| true).len(), 1);
        assert_eq!(metadata.sources(|_| true)[0].id, "internal_stock");
    }

    #[test]
    fn mcp_and_capability_context_have_equal_weight_and_distinct_source_identity() {
        let native = capability_operation(
            "peek",
            "atlas",
            "Atlas",
            "Warehouse inventory and fulfillment",
        );
        let mut remote = operation("atlas", "get", "Read one item");
        if let ToolDefinition::Builtin(tool) = &mut remote {
            tool.hints.capability_id = Some("connector".into());
            tool.hints.capability_name = Some("Logistics".into());
            tool.hints.capability_description = Some("Warehouse inventory and fulfillment".into());
        }
        let index = ToolSearchIndex::new(&[native, remote]);
        let hits = index.search("warehouse fulfillment", 8, |_| true);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].score, hits[1].score);
        assert_eq!(
            index.sources(|_| true).len(),
            2,
            "source ids must be namespaced"
        );
        let remote = index.search("logistics", 8, |_| true);
        assert_eq!(remote.len(), 1);
        assert!(remote[0].server_match);
        assert_eq!(
            remote[0].source.as_ref().unwrap().kind,
            ToolSearchSourceKind::McpServer
        );
        assert_eq!(remote[0].source.as_ref().unwrap().id, "atlas");
        assert_eq!(remote[0].score, (1.0_f64 + 1.5 / 1.5).ln());
        assert!(index.search_operations("logistics", 8, |_| true).is_empty());
    }

    #[test]
    fn hundreds_of_capability_operations_do_not_multiply_source_context() {
        let one = capability_operation("peek", "stock", "Warehouse", "Fulfillment logistics");
        let small =
            ToolSearchIndex::new(&[one.clone(), operation("cloud", "get", "Read one item")]);
        let mut definitions = vec![one, operation("cloud", "get", "Read one item")];
        definitions.extend((0..500).map(|n| {
            capability_operation(
                &format!("inspect_{n:04}"),
                "stock",
                "Warehouse",
                "Fulfillment logistics",
            )
        }));
        let large = ToolSearchIndex::new(&definitions);
        assert_eq!(large.sources(|_| true).len(), 2);
        assert_eq!(
            small.search("logistics", 8, |_| true)[0].score,
            large.search("logistics", 8, |_| true)[0].score,
        );
        assert!(large.search_operations("logistics", 8, |_| true).is_empty());
    }

    #[test]
    fn forbidden_capabilities_do_not_change_statistics_or_leak_metadata() {
        let authorized =
            capability_operation("peek", "stock", "Warehouse", "Fulfillment logistics");
        let forbidden =
            capability_operation("secret", "classified", "Secret source", "Logistics zephyr");
        let isolated = ToolSearchIndex::new(std::slice::from_ref(&authorized));
        let index = ToolSearchIndex::new(&[authorized, forbidden]);
        let allowed = |name: &str| name == "peek";
        assert_eq!(
            index.search("logistics", 8, allowed)[0].score,
            isolated.search("logistics", 8, |_| true)[0].score
        );
        assert!(index.search("zephyr", 8, allowed).is_empty());
        assert_eq!(index.sources(allowed), isolated.sources(|_| true));
        assert!(index.sources(|_| false).is_empty());
    }

    #[test]
    fn exact_operation_survives_broad_capability_context_and_context_augments_real_hits() {
        let target = capability_operation("release_invoice", "billing", "Accounting", "Payables");
        let mut definitions = vec![target.clone()];
        definitions.extend((0..500).map(|n| {
            capability_operation(
                &format!("inspect_{n:04}"),
                "control",
                "Release invoice",
                "Billing payables",
            )
        }));
        let index = ToolSearchIndex::new(&definitions);
        let exact = index.search("release invoice", 8, |_| true);
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].name, "release_invoice");
        assert!(exact[0].source.is_none());
        let hits = index.search_operations("release payables", 8, |_| true);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].tool_match);
        assert_eq!(hits[0].source.as_ref().unwrap().id, "billing");
        assert!(!hits[0].server_match);
        assert!(index.search_operations("release", 8, |_| true)[0].score < hits[0].score);
    }

    #[test]
    fn capability_context_is_bounded_and_never_indexes_system_prompt() {
        let id = "stable_identifier".repeat(30);
        let mut definition = capability_operation(
            "peek",
            &id,
            &format!("{} hiddenname", "a".repeat(256)),
            &format!("{} hiddendescription", "b".repeat(4096)),
        );
        if let ToolDefinition::Builtin(tool) = &mut definition {
            tool.hints.metadata = Some(json!({
                "system_prompt": "secretinstructions",
                "capability": {"system_prompt": "privatesystemprompt"},
            }));
        }
        let index = ToolSearchIndex::new(&[definition]);
        for query in [
            "hiddenname",
            "hiddendescription",
            "secretinstructions",
            "privatesystemprompt",
        ] {
            assert!(index.search(query, 8, |_| true).is_empty(), "{query}");
        }
        let source = &index.sources(|_| true)[0];
        assert_eq!(source.id, id, "stable ids must not be truncated");
        assert_eq!(source.name.chars().count(), 256);
    }

    #[test]
    fn capability_context_change_invalidates_registry_index_without_changing_provider_schema() {
        let original = capability_operation("peek", "stock", "Warehouse", "Turquoise");
        let updated = capability_operation("peek", "stock", "Warehouse", "Geothermal");
        assert!(crate::tool_changes::same_for_model(&original, &updated));
        let mut registry = ToolRegistry::new();
        registry.register(CatalogTool(original));
        let previous = registry.search_index();
        registry.register(CatalogTool(updated));
        let current = registry.search_index();
        assert!(!Arc::ptr_eq(&previous, &current));
        assert_eq!(previous.search("turquoise", 8, |_| true).len(), 1);
        assert!(current.search("turquoise", 8, |_| true).is_empty());
        assert_eq!(current.search("geothermal", 8, |_| true).len(), 1);
        registry.unregister("peek");
        assert!(registry.search_index().sources(|_| true).is_empty());
    }

    #[test]
    fn source_search_deduplicates_before_limiting_and_obeys_scope() {
        let mut definitions: Vec<_> = (0..100)
            .map(|n| {
                capability_operation(
                    &format!("alpha_{n:03}"),
                    "alpha",
                    "Warehouse",
                    "Fulfillment",
                )
            })
            .collect();
        definitions.push(capability_operation(
            "zulu",
            "zulu",
            "Warehouse",
            "Fulfillment",
        ));
        let index = ToolSearchIndex::new(&definitions);
        let sources = index.search_sources("warehouse", 2, |_| true);
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].id, "alpha");
        assert_eq!(sources[1].id, "zulu");
        assert_eq!(index.search_sources("warehouse", 1, |_| true).len(), 1);
        assert_eq!(
            index.search_sources("warehouse", 2, |name| name == "zulu"),
            vec![sources[1].clone()]
        );
        assert!(index.search_sources("warehouse", 2, |_| false).is_empty());
        assert!(index.search_sources("warehouse", 0, |_| true).is_empty());
        assert!(index.search_sources("unmatched", 2, |_| true).is_empty());
    }

    #[test]
    fn capability_namespace_is_source_context_and_suffix_is_an_exact_operation_alias() {
        let index = ToolSearchIndex::new(&[
            capability_operation(
                "platform_update_agent",
                "platform",
                "Platform",
                "Control plane",
            ),
            capability_operation(
                "platform_list_agents",
                "platform",
                "Platform",
                "Control plane",
            ),
        ]);
        let hits = index.search("platform", 8, |_| true);
        assert_eq!(hits.len(), 2);
        assert!(
            hits.iter()
                .all(|hit| !hit.tool_match && hit.source.is_some())
        );
        assert!(index.search_operations("platform", 8, |_| true).is_empty());
        for query in [
            "update_agent",
            "update agent",
            "update-agent",
            "platform_update_agent",
            "platform update agent",
        ] {
            let hits = index.search_operations(query, 8, |_| true);
            assert_eq!(hits.len(), 1, "{query}");
            assert_eq!(hits[0].name, "platform_update_agent");
            assert!(hits[0].score >= EXACT_NAME_BONUS);
        }
    }

    #[test]
    fn capability_operation_aliases_preserve_qualified_identity_ambiguity_and_scope() {
        let index = ToolSearchIndex::new(&[
            capability_operation(
                "platform_update_agent",
                "platform",
                "Platform",
                "Control plane",
            ),
            capability_operation(
                "analytics_update_agent",
                "analytics",
                "Analytics",
                "Reports",
            ),
            capability_operation(
                "misc_update_record",
                "misc",
                "Miscellaneous",
                "Agent records",
            ),
        ]);
        for query in ["update_agent", "update agent", "update-agent"] {
            let hits = index.search_operations(query, 8, |_| true);
            let names: HashSet<_> = hits.iter().map(|hit| hit.name.as_str()).collect();
            assert_eq!(names.len(), 2, "{query}");
            assert!(names.contains("platform_update_agent"));
            assert!(names.contains("analytics_update_agent"));
            let scoped = index.search_operations(query, 8, |name| name.starts_with("platform_"));
            assert_eq!(scoped.len(), 1);
            assert_eq!(scoped[0].name, "platform_update_agent");
        }
        for name in ["platform_update_agent", "analytics_update_agent"] {
            let hits = index.search_operations(name, 8, |_| true);
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].name, name);
        }
    }

    #[test]
    fn capability_namespace_accepts_normalized_hyphens_and_keeps_mcp_aliases() {
        let mut remote = operation("atlas", "update_agent", "Read one item");
        if let ToolDefinition::Builtin(tool) = &mut remote {
            tool.hints.capability_id = Some("mcp_atlas".into());
        }
        let index = ToolSearchIndex::new(&[
            capability_operation(
                "team_platform_update_agent",
                "team-platform",
                "Management",
                "Control plane",
            ),
            remote,
        ]);
        let hits = index.search_operations("update agent", 8, |_| true);
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().all(|hit| hit.score >= EXACT_NAME_BONUS));
        assert_eq!(
            hits[0].score, hits[1].score,
            "equivalent operation names must rank equally"
        );
        let source = index.search("team platform", 8, |_| true);
        assert_eq!(source.len(), 1);
        assert!(!source[0].tool_match);
        assert_eq!(source[0].source.as_ref().unwrap().id, "team-platform");
    }

    #[test]
    fn unowned_names_and_nonmatching_owner_prefixes_remain_operation_text() {
        let mut unowned = capability_operation(
            "platform_update_agent",
            "platform",
            "Platform",
            "Control plane",
        );
        if let ToolDefinition::Builtin(tool) = &mut unowned {
            tool.hints.capability_id = None;
            tool.hints.capability_name = None;
            tool.hints.capability_description = None;
        }
        let index = ToolSearchIndex::new(&[
            unowned,
            capability_operation(
                "platformish_update_agent",
                "platform",
                "Platform",
                "Control plane",
            ),
            capability_operation(
                "different_update_agent",
                "platform",
                "Platform",
                "Control plane",
            ),
        ]);
        let unowned = index.search_operations("platform", 8, |_| true);
        assert_eq!(unowned.len(), 1);
        assert_eq!(unowned[0].name, "platform_update_agent");
        assert!(unowned[0].source.is_none());
        for (query, expected) in [
            ("platformish", "platformish_update_agent"),
            ("different", "different_update_agent"),
        ] {
            let hits = index.search_operations(query, 8, |_| true);
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].name, expected);
        }
        let partial = index.search_operations("update agent", 8, |_| true);
        assert_eq!(partial.len(), 3);
        assert!(partial.iter().all(|hit| hit.score < EXACT_NAME_BONUS));
    }
}
