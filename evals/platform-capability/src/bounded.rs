//! Bounded native projection over the same captured catalog as CLI/eager.
//!
//! Retrieval uses the production index. This HTTP subject uses Chat Completions'
//! fallback (current tools at top level), not Framework's provider-qualified
//! inline ToolChanges. Earlier messages and schema results remain append-only;
//! additions/evictions can still invalidate the provider's tools prefix.

use std::collections::{BTreeMap, BTreeSet};

use everruns_contracts::runtime::tool_catalog::ToolSearchIndex;
use everruns_contracts::{ToolDefinition, tool_types::ToolHints};
use mira::Transcript;
use serde_json::{Value, json};

use crate::control_plane::{catalog, native_tool_definition};

const MAX_RESULTS: usize = 8;
// Matches production bounded defaults. The search tool is eager and excluded
// from these optional-schema limits. Count the actual Chat Completions schemas,
// including the invocation envelope, array delimiters and separators.
const MAX_ACTIVE_TOOLS: usize = 32;
const MAX_ACTIVE_BYTES: usize = 65_536;
const MAX_RESULT_BYTES: usize = 32_768;
// Captured from PlatformCapability::description, not an additional instruction.
const SOURCE_DESCRIPTION: &str = "Discover and execute the authorized Everruns command catalog.";

pub fn search_tool() -> Value {
    json!({"type":"function","function":{
        "name":"tool_search",
        "description":"Search Everruns platform operation names, descriptions, parameter documentation and platform source metadata. Returns at most eight full native schemas. Search results describe operations, not resource instances. Selected operations become callable on the next request; refine broad or oversized queries. Only a bounded recent set remains active.",
        "parameters":{"type":"object","properties":{"query":{"type":"string","maxLength":512}},"required":["query"],"additionalProperties":false}
    }})
}

fn bytes(value: &(impl serde::Serialize + ?Sized)) -> usize {
    serde_json::to_vec(value).expect("surface serializes").len()
}

pub struct BoundedSurface {
    definitions: BTreeMap<String, Value>,
    index: ToolSearchIndex,
    // This is a simulated visible catalog, not production authorization. It is
    // checked both during search and immediately before invocation.
    allowed: BTreeSet<String>,
    reveal_order: Vec<String>,
    active: BTreeSet<String>,
    max_tools: usize,
    max_bytes: usize,
    result_bytes: usize,
    search_calls: usize,
    disclosures: usize,
    ever_disclosed: BTreeSet<String>,
    peak_count: usize,
    peak_bytes: usize,
    changes: Vec<Value>,
}

impl BoundedSurface {
    pub fn new() -> Self {
        Self::from_catalog(
            catalog(),
            MAX_ACTIVE_TOOLS,
            MAX_ACTIVE_BYTES,
            MAX_RESULT_BYTES,
        )
    }

    fn from_catalog(
        entries: Vec<Value>,
        max_tools: usize,
        max_bytes: usize,
        result_bytes: usize,
    ) -> Self {
        let mut definitions = BTreeMap::new();
        let mut indexed = Vec::new();
        for entry in entries {
            let native = native_tool_definition(&entry);
            let function = &native["function"];
            let name = function["name"].as_str().expect("operation name");
            indexed.push(
                ToolDefinition::function(
                    name,
                    function["description"].as_str().unwrap_or_default(),
                    function["parameters"].clone(),
                )
                .with_hints(ToolHints::default().with_metadata(
                    json!({"mcp_server":{
                        "name":"everruns", "description":SOURCE_DESCRIPTION
                    }}),
                )),
            );
            definitions.insert(name.to_owned(), native);
        }
        Self {
            allowed: definitions.keys().cloned().collect(),
            definitions,
            index: ToolSearchIndex::new(&indexed),
            reveal_order: Vec::new(),
            active: BTreeSet::new(),
            max_tools,
            max_bytes,
            result_bytes,
            search_calls: 0,
            disclosures: 0,
            ever_disclosed: BTreeSet::new(),
            peak_count: 0,
            peak_bytes: 2,
            changes: Vec::new(),
        }
    }

    /// Freeze this request's callable set. A search in a batch cannot authorize
    /// another call from that same model output, before it has read the schema.
    pub fn request_tools(&mut self, message_position: usize) -> Vec<Value> {
        let previous = self.active.clone();
        let mut active = BTreeSet::new();
        let mut size = 2;
        // Production bounded projection prefers most recent reveals and emits
        // definitions in stable registry order. Repeated searches refresh recency.
        for name in self.reveal_order.iter().rev() {
            if active.len() >= self.max_tools
                || active.contains(name)
                || !self.allowed.contains(name)
            {
                continue;
            }
            let Some(definition) = self.definitions.get(name) else {
                continue;
            };
            let added = bytes(definition) + usize::from(!active.is_empty());
            if size + added <= self.max_bytes {
                size += added;
                active.insert(name.clone());
            }
        }
        self.active = active;
        self.peak_count = self.peak_count.max(self.active.len());
        self.peak_bytes = self.peak_bytes.max(size);
        if self.changes.is_empty() || previous != self.active {
            self.changes.push(json!({
                "message_position":message_position,
                "baseline": if self.changes.is_empty() { json!([search_tool()]) } else { Value::Null },
                "added":self.active.difference(&previous).map(|name| &self.definitions[name]).collect::<Vec<_>>(),
                "removed":previous.difference(&self.active).collect::<Vec<_>>(),
                "transport":"chat-completions-current-tools-fallback"
            }));
        }
        let mut tools = vec![search_tool()];
        tools.extend(
            self.active
                .iter()
                .map(|name| self.definitions[name].clone()),
        );
        tools
    }

    pub fn callable(&self, name: &str) -> bool {
        self.allowed.contains(name)
            && self.active.contains(name)
            && self.definitions.contains_key(name)
    }

    pub fn search(&mut self, arguments: &Value) -> String {
        self.search_calls += 1;
        let Some(query) = arguments.get("query").and_then(Value::as_str) else {
            return json!({"error":"query must be a string"}).to_string();
        };
        if query.len() > 512 {
            return json!({"error":"Search query exceeds 512 UTF-8 bytes; refine it."}).to_string();
        }
        if arguments.as_object().is_none_or(|object| object.len() != 1) {
            return json!({"error":"tool_search accepts only query"}).to_string();
        }
        let mut hits = self
            .index
            .search_operations(query, MAX_RESULTS, |name| self.allowed.contains(name));
        if hits.is_empty() {
            hits = self
                .index
                .search(query, MAX_RESULTS, |name| self.allowed.contains(name));
        }
        let mut matches = Vec::new();
        let mut loaded = Vec::new();
        let mut omitted = 0usize;
        let mut selected_bytes = 2;
        let output = |matches: &Vec<Value>, loaded: &Vec<String>, omitted: usize| {
            json!({"query":query,"tools":matches,"loaded":loaded,"bounded":true,
                "omitted_for_byte_budget":omitted,
                "message":"Selected schemas become native functions next step. Refine source-only or oversized matches; older operations may leave the active view."})
        };
        for hit in hits.into_iter().take(self.max_tools.min(MAX_RESULTS)) {
            if !hit.tool_match || !self.allowed.contains(&hit.name) {
                continue;
            }
            let Some(definition) = self.definitions.get(&hit.name) else {
                continue;
            };
            let size = bytes(definition) + usize::from(!loaded.is_empty());
            if selected_bytes + size > self.max_bytes {
                omitted += 1;
                continue;
            }
            let function = &definition["function"];
            matches.push(json!({"name":hit.name,"description":function["description"],
                "parameters":function["parameters"],"server_match":hit.server_match,"operation_match":hit.tool_match}));
            loaded.push(hit.name.clone());
            // Reserve the largest possible omission count while accepting each
            // match, as production does, so the final count cannot overflow.
            if bytes(&output(&matches, &loaded, MAX_RESULTS)) > self.result_bytes {
                matches.pop();
                loaded.pop();
                omitted += 1;
            } else {
                selected_bytes += size;
            }
        }
        let mut result = output(&matches, &loaded, omitted);
        if matches.is_empty() {
            // One simulated platform source; never dump the operation inventory.
            result["available_servers"] = json!(if self.allowed.is_empty() {
                vec![]
            } else {
                vec!["everruns"]
            });
        }
        if bytes(&result) > self.result_bytes {
            return json!({"error":"Search response exceeds its byte budget; use a shorter query."}).to_string();
        }
        for name in loaded {
            self.disclosures += 1;
            self.ever_disclosed.insert(name.clone());
            self.reveal_order.push(name);
        }
        result.to_string()
    }

    pub fn finish(&self, transcript: &mut Transcript) {
        for (name, value) in [
            ("search_calls", self.search_calls),
            ("disclosed_operations", self.disclosures),
            ("unique_disclosed_operations", self.ever_disclosed.len()),
            ("peak_active_operations", self.peak_count),
            ("peak_active_schema_bytes", self.peak_bytes),
        ] {
            transcript.record_metric(format!("platform.{name}"), value as f64);
        }
        transcript.metadata.insert(
            "bounded_native".into(),
            json!({
                "max_active_tools":self.max_tools,"max_active_bytes":self.max_bytes,
                "max_result_bytes":self.result_bytes,"max_search_results":MAX_RESULTS,
                "retrieval":"everruns_contracts::runtime::tool_catalog::ToolSearchIndex",
                "scope":"simulated-visible-platform-catalog",
                "transport":"chat-completions-current-tools-fallback",
                "inline_provider_tool_changes":false,
            }),
        );
        transcript
            .metadata
            .insert("tool_surface_changes".into(), json!(self.changes));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, padding: usize) -> Value {
        json!({"name":name,"description":"Read records", "input_schema":{
            "type":"object","properties":{"record":{"type":"string","description":"x".repeat(padding)}}
        }})
    }

    fn result(surface: &mut BoundedSurface, query: &str) -> Value {
        serde_json::from_str(&surface.search(&json!({"query":query}))).unwrap()
    }

    #[test]
    fn actual_platform_schemas_fit_default_limits_and_are_revealed_exactly() {
        let mut surface = BoundedSurface::new();
        assert_eq!(surface.request_tools(2), vec![search_tool()]);
        assert!(!surface.callable("update_agent"));
        for name in ["get_manager_context", "update_agent", "create_agent"] {
            let output = result(&mut surface, name);
            assert!(bytes(&output) <= MAX_RESULT_BYTES);
            assert!(
                output["loaded"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|value| value == name),
                "{name} omitted: {output}"
            );
            let selected = output["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|value| value["name"] == name)
                .unwrap();
            let expected = &surface.definitions[name]["function"];
            assert_eq!(selected["parameters"], expected["parameters"]);
        }
        let tools = surface.request_tools(8);
        assert!(
            tools
                .iter()
                .any(|tool| tool["function"]["name"] == "update_agent")
        );
        assert!(surface.callable("update_agent"));
        assert!(bytes(&tools[1..]) <= MAX_ACTIVE_BYTES);
        assert!(tools.len() <= MAX_ACTIVE_TOOLS + 1);
    }

    #[test]
    fn active_projection_is_recent_bounded_and_stably_ordered() {
        let entries = (0..40)
            .map(|index| entry(&format!("read_{index:02}"), 40))
            .collect();
        let mut surface = BoundedSurface::from_catalog(entries, 2, 2048, 2048);
        assert_eq!(surface.request_tools(2), vec![search_tool()]);
        for index in 0..100 {
            let name = format!("read_{:02}", index % 40);
            let output = result(&mut surface, &name);
            assert!(bytes(&output) <= 2048);
            assert!(output["tools"].as_array().unwrap().len() <= 2);
            let tools = surface.request_tools(index + 3);
            assert!(tools.len() <= 3);
            assert!(bytes(&tools[1..]) <= 2048);
            assert!(surface.callable(&name));
            let names: Vec<_> = tools[1..]
                .iter()
                .map(|tool| tool["function"]["name"].as_str().unwrap())
                .collect();
            assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
            assert_eq!(surface.request_tools(index + 3), tools);
        }
        result(&mut surface, "read_01");
        // Searches only affect the next request, never an already-issued batch.
        assert!(!surface.callable("read_01"));
        surface.request_tools(200);
        assert!(surface.callable("read_01"));
        assert!(!surface.callable("read_18"));
    }

    #[test]
    fn byte_omissions_and_source_browse_do_not_dump_inventory() {
        let mut surface = BoundedSurface::from_catalog(
            vec![entry("large_record", 5000), entry("small_record", 0)],
            32,
            2048,
            1024,
        );
        let output = result(&mut surface, "large_record");
        assert!(bytes(&output) <= 1024);
        assert_eq!(output["omitted_for_byte_budget"], 1);
        assert!(output["loaded"].as_array().unwrap().is_empty());
        assert_eq!(surface.request_tools(3), vec![search_tool()]);
        // Source metadata participates in production ranking but source-only
        // matches yield a refinement, never arbitrary operation disclosure.
        let output = result(&mut surface, "authorized");
        assert!(output["tools"].as_array().unwrap().is_empty());
        assert_eq!(output["available_servers"], json!(["everruns"]));
        assert!(!output.to_string().contains("large_record"));
        assert!(bytes(&output) <= 1024);
        let output = result(&mut surface, &"é".repeat(257));
        assert!(output.get("error").is_some());
        assert!(bytes(&output) <= 1024);
    }

    #[test]
    fn current_visible_scope_is_rechecked_for_search_and_execution() {
        let mut surface = BoundedSurface::from_catalog(
            vec![entry("read_allowed", 0), entry("read_forbidden", 0)],
            32,
            2048,
            2048,
        );
        surface.allowed.remove("read_forbidden");
        let output = result(&mut surface, "read_forbidden");
        assert!(!output.to_string().contains("\"name\":\"read_forbidden\""));
        assert!(!surface.callable("read_forbidden"));
        result(&mut surface, "read_allowed");
        surface.request_tools(3);
        assert!(surface.callable("read_allowed"));
        surface.allowed.remove("read_allowed");
        assert!(!surface.callable("read_allowed"));
        assert_eq!(surface.request_tools(4), vec![search_tool()]);
        assert!(!surface.callable("unknown"));
    }

    #[test]
    fn result_limit_counts_full_envelope_and_never_exceeds_eight() {
        let entries = (0..20)
            .map(|index| entry(&format!("read_record_{index:02}"), 100))
            .collect();
        let mut surface = BoundedSurface::from_catalog(entries, 32, 65536, 2048);
        let output = result(&mut surface, "read records");
        assert!(bytes(&output) <= 2048);
        assert!(output["tools"].as_array().unwrap().len() <= MAX_RESULTS);
        assert!(output["omitted_for_byte_budget"].as_u64().unwrap() > 0);
        let tools = surface.request_tools(3);
        assert_eq!(tools.len() - 1, output["loaded"].as_array().unwrap().len());
    }
}
