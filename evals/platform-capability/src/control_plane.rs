//! An in-process control plane for offline runs.
//!
//! The live subject drives a real server, which is the right way to measure
//! authorization, persistence and the worker adapter. It is the wrong way to
//! measure whether a model can find and spell a command: that needs a database,
//! a scheduler and a web server to be running before a single case can execute,
//! which is why these cases were not run.
//!
//! So this is the other half. It fakes exactly one thing — persistence — and
//! takes everything else from the artifacts the server generates:
//!
//! * `catalog.json`, every command in inventory with its real description and
//!   schema, so `discover` returns the text a model actually reads;
//! * `commands.json`, the shared CLI contract, so `everruns agents list --limit`
//!   is parsed by the same `clap::Command` the server builds;
//! * `harness.json`, the shipped system prompt and tool schemas.
//!
//! What it therefore cannot measure: authorization, real validation beyond
//! argument shape, anything about persistence, and any behaviour that depends
//! on a command's real output values. A case that needs those belongs on the
//! live subject, and the two are selected by env rather than merged.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::{Map, Value, json};

/// The command surface, as the server generated it.
const CATALOG: &str = include_str!("../catalog.json");
/// The shipped harness: system prompt and tool schemas.
const HARNESS: &str = include_str!("../harness.json");

pub struct FakeControlPlane {
    catalog: Vec<Value>,
    store: Mutex<Store>,
}

struct Store {
    /// Entity family (`agents`, `harnesses`, …) to rows.
    families: BTreeMap<String, Vec<Value>>,
    next_id: u32,
}

/// The shipped system prompt, so an offline run grades the prompt that ships.
pub fn system_prompt() -> String {
    serde_json::from_str::<Value>(HARNESS)
        .ok()
        .and_then(|h| h["system_prompt"].as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// The platform tools, in the provider's function-calling shape.
pub fn tool_definitions() -> Vec<Value> {
    let harness: Value = serde_json::from_str(HARNESS).expect("harness.json parses");
    let described = |name: &str| -> String {
        // The tool descriptions live in the platform crate as private consts.
        // These restate their contract in one line each; the schemas, which are
        // what the model is actually held to, are the generated ones.
        match name {
            "discover" => {
                "Search the platform operation catalog. Finds operations, not resource instances."
            }
            "query" => "Run a bash script whose builtins are the read-only platform operations.",
            _ => {
                "Run a bash script whose builtins are the platform operations, including mutations."
            }
        }
        .to_string()
    };
    harness["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .map(|tool| {
                    let name = tool["name"].as_str().unwrap_or_default();
                    json!({
                        "type": "function",
                        "function": {
                            "name": name,
                            "description": described(name),
                            "parameters": tool["schema"],
                        }
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Default for FakeControlPlane {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeControlPlane {
    pub fn new() -> Self {
        Self {
            catalog: serde_json::from_str(CATALOG).expect("catalog.json parses"),
            store: Mutex::new(Store::seeded()),
        }
    }

    /// Run one tool call, returning the text the model sees.
    pub fn call(&self, tool: &str, arguments: &Value) -> String {
        match tool {
            "discover" => self.discover(arguments),
            "query" => self.script(arguments, true),
            "execute" => self.script(arguments, false),
            other => format!("unknown tool `{other}`"),
        }
    }

    fn discover(&self, arguments: &Value) -> String {
        let all = arguments["all"].as_bool().unwrap_or(false);
        let query = arguments["query"].as_str().unwrap_or("").to_lowercase();

        if all {
            let mut by_category: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for entry in &self.catalog {
                by_category
                    .entry(entry["category"].as_str().unwrap_or(""))
                    .or_default()
                    .push(entry["name"].as_str().unwrap_or(""));
            }
            return json!({ "operations_by_category": by_category }).to_string();
        }

        let mut matches: Vec<&Value> = self
            .catalog
            .iter()
            .filter(|entry| {
                let name = entry["name"].as_str().unwrap_or("").to_lowercase();
                let description = entry["description"].as_str().unwrap_or("").to_lowercase();
                let cli = entry["cli"].as_str().unwrap_or("").to_lowercase();
                query.split_whitespace().all(|term| {
                    name.contains(term) || description.contains(term) || cli.contains(term)
                })
            })
            .collect();

        // An exact name or spelling wins outright, so a lookup is a lookup.
        if let Some(exact) = self.catalog.iter().find(|entry| {
            entry["name"]
                .as_str()
                .unwrap_or("")
                .eq_ignore_ascii_case(&query)
                || entry["cli"]
                    .as_str()
                    .unwrap_or("")
                    .eq_ignore_ascii_case(&query)
        }) {
            matches = vec![exact];
        }
        matches.truncate(12);

        // Schemas only for a single hit, matching the real surface's rule that
        // broad searches must not blow the context window.
        let include_schemas = matches.len() <= 1;
        let operations: Vec<Value> = matches
            .into_iter()
            .map(|entry| {
                let mut value = json!({
                    "name": entry["name"],
                    "category": entry["category"],
                    "description": entry["description"],
                    "read_only": entry["read_only"],
                });
                if let Some(cli) = entry["cli"].as_str() {
                    value["cli"] = json!(format!("everruns {cli}"));
                    value["help"] = json!(format!("everruns {cli} --help"));
                }
                if include_schemas {
                    value["input_schema"] = entry["input_schema"].clone();
                }
                value
            })
            .collect();

        json!({ "operations": operations, "count": operations.len() }).to_string()
    }

    /// Run a script's platform invocations and return their combined stdout.
    fn script(&self, arguments: &Value, read_only: bool) -> String {
        let Some(script) = arguments["commands"].as_str() else {
            return "commands is required".to_string();
        };

        let mut output = String::new();
        for statement in statements(script) {
            let result = self.statement(&statement, read_only);
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&result);
        }
        if output.is_empty() {
            "".to_string()
        } else {
            output
        }
    }

    fn statement(&self, statement: &str, read_only: bool) -> String {
        let words = shell_split(statement);
        if words.is_empty() {
            return String::new();
        }

        let contracts = everruns_cli_contract::commands();
        let (wire_name, args) = if words[0] == "everruns" {
            // Longest spelling wins, exactly as the tree resolves it.
            let rest = &words[1..];
            if rest.is_empty() || rest[0] == "--help" || rest[0] == "-h" {
                return self.root_help();
            }
            let mut found = None;
            for take in (1..=rest.len().min(4)).rev() {
                let spelling = rest[..take].join(" ");
                if let Some(c) = contracts.iter().find(|c| c.spelling() == spelling) {
                    found = Some((c, &rest[take..]));
                    break;
                }
            }
            let Some((contract, tail)) = found else {
                return self.node_help(rest);
            };

            // The real parser, so an unknown flag fails here for the same
            // reason it fails in production.
            let display = format!("everruns {}", contract.spelling());
            let parser = contract.clap_command(&display);
            let argv = std::iter::once(display.clone()).chain(tail.iter().cloned());
            match parser.try_get_matches_from(argv) {
                Ok(matches) => (
                    contract.wire_name.clone(),
                    everruns_cli_contract::params_from(contract, &matches),
                ),
                Err(error) => return error.render().to_string(),
            }
        } else {
            (words[0].clone(), flat_args(&words[1..]))
        };

        let Some(entry) = self
            .catalog
            .iter()
            .find(|e| e["name"].as_str() == Some(wire_name.as_str()))
        else {
            return format!("{wire_name}: command not found");
        };

        if read_only && !entry["read_only"].as_bool().unwrap_or(false) {
            return format!(
                "{wire_name}: not available in query; it is a mutation, so use execute"
            );
        }

        self.store
            .lock()
            .expect("store")
            .run(&wire_name, &args, entry)
    }

    fn root_help(&self) -> String {
        let mut nouns: Vec<&str> = self
            .catalog
            .iter()
            .filter_map(|e| e["cli"].as_str())
            .filter_map(|cli| cli.split(' ').next())
            .collect();
        nouns.sort_unstable();
        nouns.dedup();
        format!(
            "Usage: everruns <command> [--flags]\n\nCommands:\n{}\n\nRun `everruns <command> --help` for its verbs.\n",
            nouns
                .iter()
                .map(|n| format!("  {n}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }

    fn node_help(&self, words: &[String]) -> String {
        // Report against the deepest prefix that exists, so a wrong guess is
        // answered with the real neighbours.
        let contracts = everruns_cli_contract::commands();
        for take in (1..=words.len()).rev() {
            let prefix = words[..take].join(" ");
            let children: Vec<String> = contracts
                .iter()
                .filter_map(|c| {
                    c.spelling()
                        .strip_prefix(&format!("{prefix} "))
                        .map(|rest| rest.split(' ').next().unwrap_or(rest).to_string())
                })
                .collect();
            if !children.is_empty() {
                let mut children = children;
                children.sort_unstable();
                children.dedup();
                return format!(
                    "Usage: everruns {prefix} <command> [--flags]\n\nCommands:\n{}\n",
                    children
                        .iter()
                        .map(|c| format!("  {c}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }
        format!(
            "unknown command `everruns {}`\n\n{}",
            words.join(" "),
            self.root_help()
        )
    }
}

impl Store {
    fn seeded() -> Self {
        let mut families: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        families.insert(
            "agents".into(),
            vec![
                json!({"id":"agent_01triage","name":"Triage","description":"Triages incoming issues.","harness_id":"harness_01generic","default_model_id":"model_01terra","status":"active","ui_link":"/agents/agent_01triage"}),
                json!({"id":"agent_01docs","name":"Docs Bot","description":"Answers product questions.","harness_id":"harness_01generic","default_model_id":"model_01terra","status":"active","ui_link":"/agents/agent_01docs"}),
            ],
        );
        families.insert(
            "harnesses".into(),
            vec![
                json!({"id":"harness_01generic","name":"Generic","description":"Filesystem, bash, storage, schedules, compaction.","ui_link":"/harnesses/harness_01generic"}),
                json!({"id":"harness_01platform","name":"platform-chat","description":"Platform administration chat.","ui_link":"/harnesses/harness_01platform"}),
                json!({"id":"harness_01base","name":"base","description":"Minimal base harness.","ui_link":"/harnesses/harness_01base"}),
            ],
        );
        families.insert(
            "models".into(),
            vec![
                json!({"id":"model_01terra","model_id":"gpt-5.6-terra","name":"GPT-5.6 Terra","provider":"openai"}),
                json!({"id":"model_01muse","model_id":"muse-spark-1.3","name":"Muse Spark 1.3","provider":"meta"}),
            ],
        );
        families.insert(
            "skills".into(),
            vec![
                json!({"id":"skill_01review","name":"code-review","description":"Reviews a diff against the house style.","status":"active"}),
                json!({"id":"skill_01release","name":"release-checklist","description":"Walks the release steps.","status":"active"}),
            ],
        );
        families.insert(
            "mcp_servers".into(),
            vec![json!({"id":"mcp_01github","name":"github","url":"https://api.example.com/mcp","status":"active"})],
        );
        families.insert("sessions".into(), vec![]);
        families.insert("messages".into(), vec![]);
        families.insert(
            "capabilities".into(),
            vec![
                json!({"id":"bashkit_shell","name":"Bash","status":"available"}),
                json!({"id":"filesystem","name":"Filesystem","status":"available"}),
            ],
        );
        families.insert("plugins".into(), vec![]);
        families.insert("connection_providers".into(), vec![]);
        families.insert("user_connections".into(), vec![]);
        families.insert("agent_versions".into(), vec![]);
        families.insert("agent_triggers".into(), vec![]);

        Self {
            families,
            next_id: 1,
        }
    }

    /// Execute one resolved command against the in-memory rows.
    ///
    /// Behaviour follows the command's name and HTTP method rather than a
    /// per-command implementation: `list_*` pages a family, `get_*` looks one
    /// up, `create_*` inserts, `update_*` merges, `delete_*` archives. That
    /// covers the surface uniformly, and a command whose family is not seeded
    /// answers with an empty page rather than an error, so a case never fails
    /// on the fake's gaps instead of on the model.
    fn run(&mut self, wire_name: &str, args: &Value, entry: &Value) -> String {
        let family = family_of(wire_name, entry);
        let args = args.as_object().cloned().unwrap_or_default();

        if let Some(rest) = wire_name.strip_prefix("list_") {
            let rows = self.families.entry(family.clone()).or_default().clone();
            let search = args
                .get("search")
                .and_then(Value::as_str)
                .map(str::to_lowercase);
            let filtered: Vec<Value> = rows
                .into_iter()
                .filter(|row| {
                    search.as_ref().is_none_or(|needle| {
                        row["name"]
                            .as_str()
                            .unwrap_or("")
                            .to_lowercase()
                            .contains(needle)
                            || row["description"]
                                .as_str()
                                .unwrap_or("")
                                .to_lowercase()
                                .contains(needle)
                    })
                })
                .collect();
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(filtered.len().max(1) as u64) as usize;
            let total = filtered.len();
            let page: Vec<Value> = filtered.into_iter().take(limit).collect();
            let _ = rest;
            return json!({ "data": page, "total": total, "offset": 0, "limit": limit })
                .to_string();
        }

        if wire_name.starts_with("get_") {
            let id = first_id(&args);
            let rows = self.families.entry(family.clone()).or_default();
            return match rows
                .iter()
                .find(|row| Some(row["id"].as_str().unwrap_or("")) == id.as_deref())
            {
                Some(row) => row.to_string(),
                None => format!(
                    "{wire_name}: no {family} with id {}",
                    id.unwrap_or_else(|| "<missing>".into())
                ),
            };
        }

        if wire_name.starts_with("create_") {
            let id = format!("{}_{:02}new", family.trim_end_matches('s'), self.next_id);
            self.next_id += 1;
            let mut row = Map::new();
            row.insert("id".into(), json!(id));
            for (key, value) in &args {
                row.insert(key.clone(), value.clone());
            }
            row.entry("ui_link".to_string())
                .or_insert(json!(format!("/{family}/{id}")));
            let row = Value::Object(row);
            self.families.entry(family).or_default().push(row.clone());
            return row.to_string();
        }

        if wire_name.starts_with("update_") || wire_name.starts_with("upsert_") {
            let id = first_id(&args);
            let rows = self.families.entry(family.clone()).or_default();
            if let Some(row) = rows
                .iter_mut()
                .find(|row| Some(row["id"].as_str().unwrap_or("")) == id.as_deref())
            {
                if let Some(object) = row.as_object_mut() {
                    for (key, value) in &args {
                        object.insert(key.clone(), value.clone());
                    }
                }
                return row.to_string();
            }
            return format!(
                "{wire_name}: no {family} with id {}",
                id.unwrap_or_else(|| "<missing>".into())
            );
        }

        if wire_name.starts_with("delete_") || wire_name.starts_with("destroy_") {
            let id = first_id(&args);
            let rows = self.families.entry(family.clone()).or_default();
            let before = rows.len();
            rows.retain(|row| Some(row["id"].as_str().unwrap_or("")) != id.as_deref());
            return json!({ "deleted": before != rows.len(), "id": id }).to_string();
        }

        // Anything else acknowledges with its arguments, which is enough for a
        // model to proceed and honest about having done nothing real.
        json!({ "ok": true, "operation": wire_name, "arguments": args }).to_string()
    }
}

/// Entity family for a command, from its REST path when it has one.
fn family_of(wire_name: &str, entry: &Value) -> String {
    // The last non-placeholder segment, not the first: `/v1/agents/{id}/versions`
    // is the versions family, and taking `agents` there filed a created version
    // among the agents and made `list_agent_versions` return agents. A model
    // reading that back thrashes, and the case fails on the fake rather than on
    // the model.
    if let Some(path) = entry["path"].as_str() {
        if let Some(segment) = path
            .trim_start_matches('/')
            .split('/')
            .skip(1)
            .filter(|segment| !segment.is_empty() && !segment.starts_with('{'))
            .last()
        {
            return segment.replace('-', "_");
        }
    }
    wire_name
        .split_once('_')
        .map(|(_, rest)| rest.to_string())
        .unwrap_or_else(|| wire_name.to_string())
}

fn first_id(args: &Map<String, Value>) -> Option<String> {
    ["id", "session_id", "agent_id", "skill_id"]
        .iter()
        .find_map(|key| {
            args.get(*key)
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

/// Split a script into the statements worth running.
///
/// Pipelines are truncated at the first `|`: the fake does not interpret `jq`,
/// and returning the command's own JSON is more useful to the model than an
/// error about a filter that is not the thing being measured.
fn statements(script: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in script.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        for part in line.split(&[';'][..]) {
            for part in part.split("&&") {
                let part = part.split('|').next().unwrap_or("").trim();
                let part = part.split('>').next().unwrap_or("").trim();
                // `name=$(everruns …)` runs the substitution.
                let part = match part.find("$(") {
                    Some(start) => part[start + 2..].trim_end_matches(')').trim(),
                    None => part,
                };
                if !part.is_empty() {
                    out.push(part.to_string());
                }
            }
        }
    }
    out
}

/// Split one statement into words, honouring quotes.
fn shell_split(statement: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;

    for ch in statement.chars() {
        match (quote, ch) {
            (Some(open), _) if ch == open => quote = None,
            (Some(_), _) => current.push(ch),
            (None, '\'' | '"') => {
                quote = Some(ch);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, c) => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(current);
    }
    words
}

/// Parse `--flag value` pairs for a flat wire name, which has no contract.
fn flat_args(words: &[String]) -> Value {
    let mut object = Map::new();
    let mut index = 0;
    while index < words.len() {
        let Some(name) = words[index].strip_prefix("--") else {
            index += 1;
            continue;
        };
        if let Some((key, value)) = name.split_once('=') {
            object.insert(key.to_string(), scalar(value));
            index += 1;
            continue;
        }
        match words.get(index + 1) {
            Some(value) if !value.starts_with("--") => {
                object.insert(name.to_string(), scalar(value));
                index += 2;
            }
            _ => {
                object.insert(name.to_string(), Value::Bool(true));
                index += 1;
            }
        }
    }
    Value::Object(object)
}

fn scalar(value: &str) -> Value {
    if let Ok(parsed) = serde_json::from_str::<Value>(value) {
        if !parsed.is_string() {
            return parsed;
        }
    }
    Value::String(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane() -> FakeControlPlane {
        FakeControlPlane::new()
    }

    #[test]
    fn a_tree_spelling_runs_and_pages_a_family() {
        let out = plane().call("query", &json!({ "commands": "everruns agents list" }));
        let value: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(value["total"], 2, "{out}");
        assert_eq!(value["data"][0]["name"], "Triage");
    }

    /// The whole point of the fake: an absent flag is rejected by the real
    /// parser, so an offline case measures the same failure production has.
    #[test]
    fn an_absent_flag_is_rejected_by_the_real_parser() {
        let out = plane().call(
            "query",
            &json!({ "commands": "everruns skills list --limit 20" }),
        );
        assert!(out.contains("unexpected argument"), "{out}");
        assert!(out.contains("--limit"), "{out}");
    }

    #[test]
    fn a_flat_wire_name_still_works() {
        let out = plane().call("query", &json!({ "commands": "list_harnesses" }));
        assert!(out.contains("Generic"), "{out}");
    }

    #[test]
    fn query_refuses_a_mutation_and_execute_runs_it() {
        let refused = plane().call(
            "query",
            &json!({ "commands": "everruns agents create --name Bot --system-prompt Hi" }),
        );
        assert!(refused.contains("use execute"), "{refused}");

        let plane = plane();
        let created = plane.call(
            "execute",
            &json!({ "commands": "everruns agents create --name Bot --system-prompt Hi" }),
        );
        assert!(created.contains("\"name\":\"Bot\""), "{created}");
        // And it persists for the rest of the turn, so a follow-up read works.
        let listed = plane.call("query", &json!({ "commands": "everruns agents list" }));
        assert!(listed.contains("Bot"), "{listed}");
    }

    #[test]
    fn a_bare_word_reaches_the_positional_field() {
        let out = plane().call(
            "query",
            &json!({ "commands": "everruns agents get agent_01triage" }),
        );
        assert!(out.contains("Triage"), "{out}");
    }

    #[test]
    fn discover_finds_an_operation_and_carries_its_spelling() {
        let out = plane().call("discover", &json!({ "query": "list_harnesses" }));
        assert!(out.contains("list_harnesses"), "{out}");
        assert!(
            out.contains("input_schema"),
            "an exact lookup carries schemas: {out}"
        );
    }

    /// A broad search must not expand every schema, which is the rule the real
    /// surface follows to keep discovery affordable.
    #[test]
    fn a_broad_search_omits_schemas() {
        let out = plane().call("discover", &json!({ "query": "agent" }));
        assert!(!out.contains("input_schema"), "{out}");
    }

    #[test]
    fn help_lists_the_nouns_and_a_nodes_verbs() {
        let root = plane().call("query", &json!({ "commands": "everruns --help" }));
        assert!(root.contains("agents"), "{root}");
        let node = plane().call("query", &json!({ "commands": "everruns agents" }));
        assert!(node.contains("versions") && node.contains("list"), "{node}");
    }

    #[test]
    fn a_pipeline_runs_its_command_and_ignores_the_filter() {
        let out = plane().call(
            "query",
            &json!({ "commands": "everruns agents list | jq -r '.data[].id'" }),
        );
        assert!(out.contains("agent_01triage"), "{out}");
    }

    #[test]
    fn several_statements_run_in_order() {
        let plane = plane();
        let out = plane.call(
            "execute",
            &json!({ "commands": "everruns agents create --name A --system-prompt X\neverruns agents list" }),
        );
        assert!(out.contains("\"name\":\"A\""), "{out}");
        assert!(out.matches("Triage").count() >= 1, "{out}");
    }
}

#[cfg(test)]
mod family_tests {
    use super::*;

    fn family(path: &str, name: &str) -> String {
        family_of(name, &json!({ "path": path }))
    }

    /// A nested collection is its own family. Filing `create_agent_version`
    /// under `agents` made `list_agent_versions` return agents, which is worse
    /// than an error: the model believes it and keeps going.
    #[test]
    fn a_nested_collection_is_its_own_family() {
        assert_eq!(
            family("/v1/agents/{agent_id}/versions", "create_agent_version"),
            "versions"
        );
        assert_eq!(family("/v1/agents", "list_agents"), "agents");
        assert_eq!(family("/v1/agents/{id}", "get_agent"), "agents");
        assert_eq!(
            family("/v1/sessions/{session_id}/messages", "create_message"),
            "messages"
        );
    }

    #[test]
    fn a_created_version_is_listed_by_its_own_command() {
        let plane = FakeControlPlane::new();
        plane.call(
            "execute",
            &json!({ "commands": "create_agent_version --agent_id agent_01triage --req {\"summary\":\"before\"}" }),
        );
        let listed = plane.call(
            "query",
            &json!({ "commands": "list_agent_versions --agent_id agent_01triage" }),
        );
        assert!(listed.contains("before"), "{listed}");
        // And the agents family is untouched.
        let agents = plane.call("query", &json!({ "commands": "everruns agents list" }));
        let value: Value = serde_json::from_str(&agents).expect("json");
        assert_eq!(value["total"], 2, "{agents}");
    }
}
