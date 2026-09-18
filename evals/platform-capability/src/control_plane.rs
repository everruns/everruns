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
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};

/// The command surface, as the server generated it.
const CATALOG: &str = include_str!("../catalog.json");
/// Platform Chat v1: system prompt and the three platform tool schemas.
const HARNESS: &str = include_str!("../harness.json");
/// Platform Chat v2: system prompt and the one `bash` tool schema.
const HARNESS_V2: &str = include_str!("../harness-v2.json");
/// Node help, rendered by the shipped tree. Discovery in v2 is `--help` and
/// almost nothing else, so hand-rolling it would grade the hand-rolled text.
const HELP: &str = include_str!("../help.json");

/// Which shipped harness an offline run reproduces.
///
/// The A/B is exactly this: the same dataset, the same fake control plane and
/// the same model, differing only in the surface the model is handed. v1 gets
/// `discover`/`query`/`execute`; v2 gets one `bash` over a real interpreter in
/// which `everruns` is a builtin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Harness {
    PlatformChat,
    PlatformChatV2,
}

impl Harness {
    /// Selected with `EVERRUNS_EVAL_HARNESS`; v1 stays the default, as it is
    /// the surface that ships to every org.
    pub fn from_env() -> Self {
        match std::env::var("EVERRUNS_EVAL_HARNESS")
            .unwrap_or_default()
            .trim()
        {
            "platform-chat-v2" | "v2" => Self::PlatformChatV2,
            _ => Self::PlatformChat,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::PlatformChat => "platform-chat",
            Self::PlatformChatV2 => "platform-chat-v2",
        }
    }

    fn artifact(self) -> &'static str {
        match self {
            Self::PlatformChat => HARNESS,
            Self::PlatformChatV2 => HARNESS_V2,
        }
    }
}

pub struct FakeControlPlane {
    catalog: Arc<Vec<Value>>,
    store: Arc<Mutex<Store>>,
    /// Present only for v2, where the model's script is run rather than
    /// approximated. One instance per case, so `cd`, variables and files
    /// persist across the turn's tool calls the way a session's shell does.
    shell: Option<tokio::sync::Mutex<bashkit::Bash>>,
}

struct Store {
    /// Entity family (`agents`, `harnesses`, …) to rows.
    families: BTreeMap<String, Vec<Value>>,
    next_id: u32,
}

/// The shipped system prompt, so an offline run grades the prompt that ships.
pub fn system_prompt(harness: Harness) -> String {
    serde_json::from_str::<Value>(harness.artifact())
        .ok()
        .and_then(|h| h["system_prompt"].as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

/// The harness's tools, in the provider's function-calling shape.
pub fn tool_definitions(harness: Harness) -> Vec<Value> {
    let harness: Value = serde_json::from_str(harness.artifact()).expect("harness artifact parses");
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
                    // v2's artifact carries the shipped description verbatim;
                    // v1's tool descriptions are private consts, so `described`
                    // restates their contract in one line.
                    let description = tool["description"]
                        .as_str()
                        .map_or_else(|| described(name), ToOwned::to_owned);
                    json!({
                        "type": "function",
                        "function": {
                            "name": name,
                            "description": description,
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
        Self::new(Harness::PlatformChat)
    }
}

impl FakeControlPlane {
    pub fn new(harness: Harness) -> Self {
        let catalog: Arc<Vec<Value>> =
            Arc::new(serde_json::from_str(CATALOG).expect("catalog.json parses"));
        let store = Arc::new(Mutex::new(Store::seeded()));
        let shell = match harness {
            Harness::PlatformChat => None,
            Harness::PlatformChatV2 => Some(tokio::sync::Mutex::new(build_shell(
                catalog.clone(),
                store.clone(),
            ))),
        };
        Self {
            catalog,
            store,
            shell,
        }
    }

    /// Run one tool call, returning the text the model sees.
    pub async fn call(&self, tool: &str, arguments: &Value) -> String {
        match tool {
            "discover" => self.discover(arguments),
            "query" => self.script(arguments, true),
            "execute" => self.script(arguments, false),
            "bash" => self.bash(arguments).await,
            other => format!("unknown tool `{other}`"),
        }
    }

    /// v2's only tool: hand the script to a real interpreter.
    ///
    /// Nothing here inspects the script. That is the point: v1's arm has to
    /// split statements itself and got loops wrong, while here `for`, `|`, `>`
    /// and `jq` are the shell's, so a failure is the model's or the contract's.
    async fn bash(&self, arguments: &Value) -> String {
        let Some(script) = arguments["commands"].as_str() else {
            return "Missing required parameter: commands".to_string();
        };
        let Some(shell) = &self.shell else {
            return "bash: not available in this harness".to_string();
        };
        let mut shell = shell.lock().await;
        match shell.exec(script).await {
            Ok(result) => {
                let mut output = result.stdout.text_lossy().into_owned();
                let stderr = result.stderr.text_lossy();
                if !stderr.trim().is_empty() {
                    if !output.is_empty() && !output.ends_with('\n') {
                        output.push('\n');
                    }
                    output.push_str(&stderr);
                }
                if result.exit_code != 0 {
                    output.push_str(&format!("\n[exit {}]", result.exit_code));
                }
                output
            }
            Err(error) => format!("bash: {error}"),
        }
    }

    fn discover(&self, arguments: &Value) -> String {
        let all = arguments["all"].as_bool().unwrap_or(false);
        let query = arguments["query"].as_str().unwrap_or("").to_lowercase();

        if all {
            let mut by_category: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
            for entry in self.catalog.iter() {
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
        if words[0] == "everruns" {
            return everruns(&self.catalog, &self.store, &words[1..], read_only).0;
        }
        // A flat wire name has no contract, so its flags are read positionally.
        run_wire_name(
            &self.catalog,
            &self.store,
            &words[0],
            &flat_args(&words[1..]),
            read_only,
        )
        .0
    }
}

/// Run `everruns <args…>` and return its output and exit code.
///
/// Shared by both arms: v1 reaches it through `query`/`execute`, v2 through the
/// shell builtin. One implementation is what makes the A/B a comparison of
/// surfaces rather than of two different fakes.
fn everruns(
    catalog: &[Value],
    store: &Mutex<Store>,
    args: &[String],
    read_only: bool,
) -> (String, i32) {
    let contracts = everruns_cli_contract::commands();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        return (root_help(), 0);
    }

    // Longest spelling wins, exactly as the tree resolves it.
    let mut found = None;
    for take in (1..=args.len().min(4)).rev() {
        let spelling = args[..take].join(" ");
        if let Some(contract) = contracts.iter().find(|c| c.spelling() == spelling) {
            found = Some((contract, &args[take..]));
            break;
        }
    }
    let Some((contract, tail)) = found else {
        return (node_help(args), 1);
    };

    // The real parser, so an unknown flag fails here for the same reason it
    // fails in production, and `--help` renders the shipped help.
    let display = format!("everruns {}", contract.spelling());
    let parser = contract.clap_command(&display);
    let argv = std::iter::once(display.clone()).chain(tail.iter().cloned());
    match parser.try_get_matches_from(argv) {
        Ok(matches) => {
            let params = everruns_cli_contract::params_from(contract, &matches);
            run_wire_name(catalog, store, &contract.wire_name, &params, read_only)
        }
        Err(error) => {
            let code = i32::from(error.use_stderr());
            (error.render().to_string(), code)
        }
    }
}

/// Run one resolved operation against the in-memory rows.
fn run_wire_name(
    catalog: &[Value],
    store: &Mutex<Store>,
    wire_name: &str,
    args: &Value,
    read_only: bool,
) -> (String, i32) {
    let Some(entry) = catalog
        .iter()
        .find(|e| e["name"].as_str() == Some(wire_name))
    else {
        return (format!("{wire_name}: command not found"), 127);
    };

    if read_only && !entry["read_only"].as_bool().unwrap_or(false) {
        return (
            format!("{wire_name}: not available in query; it is a mutation, so use execute"),
            1,
        );
    }

    (store.lock().expect("store").run(wire_name, args, entry), 0)
}

/// The help the shipped tree renders for a node, root included.
///
/// A leaf's help is clap's, generated from the same contract on both sides, so
/// only these travel as an artifact.
fn node_help_text(path: &str) -> Option<String> {
    static HELP_NODES: std::sync::OnceLock<BTreeMap<String, String>> = std::sync::OnceLock::new();
    HELP_NODES
        .get_or_init(|| serde_json::from_str(HELP).expect("help.json parses"))
        .get(path)
        .cloned()
}

fn root_help() -> String {
    node_help_text("").unwrap_or_else(|| "everruns\n  no commands available\n".to_string())
}

/// Report against the deepest prefix that exists, so a wrong guess is answered
/// with the real neighbours, exactly as the shipped tree does.
fn node_help(words: &[String]) -> String {
    for take in (1..=words.len()).rev() {
        let prefix = words[..take].join(" ");
        if let Some(text) = node_help_text(&prefix) {
            if take == words.len() {
                return text;
            }
            return format!(
                "unknown command `{}` under `everruns {prefix}`\n\n{text}",
                words[take]
            );
        }
    }
    format!(
        "unknown command `everruns {}`\n\n{}",
        words.join(" "),
        root_help()
    )
}

/// Whether a script invokes any operation the catalog marks as a mutation.
///
/// Both arms are graded by one dataset, and that dataset names `query` and
/// `execute`. v2 has a single tool for both, so what a `bash` call counts as is
/// decided by what it runs: a script that only reads is that arm's `query`, and
/// one that writes is its `execute`. Without this the A/B would compare a
/// surface against a scorer written for the other surface.
pub fn script_mutates(script: &str) -> bool {
    static MUTATIONS: std::sync::OnceLock<regex::RegexSet> = std::sync::OnceLock::new();
    MUTATIONS
        .get_or_init(|| {
            let catalog: Vec<Value> = serde_json::from_str(CATALOG).expect("catalog.json parses");
            let read_only: std::collections::HashSet<&str> = catalog
                .iter()
                .filter(|entry| entry["read_only"].as_bool().unwrap_or(false))
                .filter_map(|entry| entry["name"].as_str())
                .collect();
            let mut patterns: Vec<String> = Vec::new();
            for entry in &catalog {
                let Some(name) = entry["name"].as_str() else {
                    continue;
                };
                if read_only.contains(name) {
                    continue;
                }
                // The flat wire name, and the tree spelling with any run of
                // whitespace between its words, since a model may wrap a long
                // command line.
                patterns.push(format!(r"\b{}\b", regex::escape(name)));
                if let Some(cli) = entry["cli"].as_str() {
                    let spelling = cli
                        .split_whitespace()
                        .map(regex::escape)
                        .collect::<Vec<_>>()
                        .join(r"\s+");
                    patterns.push(format!(r"\beverruns\s+{spelling}\b"));
                }
            }
            regex::RegexSet::new(&patterns).expect("mutation patterns compile")
        })
        .is_match(&without_help(script))
}

/// Drop lines that only ask a command to describe itself.
///
/// `everruns agents create --help` names a mutation and performs none. Counting
/// it as one would fail a read-only case for the very behaviour the CLI cases
/// reward: reading the help before guessing a flag.
fn without_help(script: &str) -> String {
    script
        .lines()
        .filter(|line| {
            let words = line.split_whitespace().collect::<Vec<_>>();
            !words.iter().any(|word| *word == "--help" || *word == "-h")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `everruns` builtin: the same resolution the tool path uses, wired into
/// the interpreter so the model's own script drives it.
struct EverrunsBuiltin {
    catalog: Arc<Vec<Value>>,
    store: Arc<Mutex<Store>>,
}

#[async_trait::async_trait]
impl bashkit::Builtin for EverrunsBuiltin {
    async fn execute(
        &self,
        ctx: bashkit::BuiltinContext<'_>,
    ) -> bashkit::Result<bashkit::ExecResult> {
        // Mutations are the point of a chat that administers a platform, so the
        // shell surface has no read-only gate; v1's `query`/`execute` split is
        // a property of having two tools, not of the catalog.
        let (output, code) = everruns(&self.catalog, &self.store, ctx.args, false);
        Ok(if code == 0 {
            bashkit::ExecResult::ok(output)
        } else {
            bashkit::ExecResult::err(output, code)
        })
    }

    fn llm_hint(&self) -> Option<&'static str> {
        Some(
            "everruns <noun> <verb> --flags: run a platform operation; --help lists nouns and verbs",
        )
    }
}

/// The v2 session shell: the product's namespace, with `everruns` in it.
fn build_shell(catalog: Arc<Vec<Value>>, store: Arc<Mutex<Store>>) -> bashkit::Bash {
    let fs = bashkit::InMemoryFs::new();
    for dir in [
        "/workspace",
        "/workspace/docs",
        "/memory",
        "/memory/shared",
        "/memory/user",
        "/outputs",
    ] {
        fs.add_dir(dir, 0o755);
    }
    // One page of documentation, so a case that is told to consult `/docs`
    // finds something rather than an empty tree. The fake cannot ship the real
    // corpus, and a case that grades documentation answers belongs on the live
    // subject.
    fs.add_file(
        "/workspace/docs/harnesses.md",
        b"# Harnesses\n\nA harness is a reusable bundle of capabilities and a system prompt.\n          The built-in `Generic` harness carries file system, bash, storage, schedules and\n          context compaction, and is the default when a session names no harness.\n",
        0o644,
    );

    bashkit::Bash::builder()
        .fs(Arc::new(fs))
        .cwd("/workspace")
        .builtin("everruns", Box::new(EverrunsBuiltin { catalog, store }))
        .build()
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
        FakeControlPlane::new(Harness::PlatformChat)
    }

    #[tokio::test]
    async fn a_tree_spelling_runs_and_pages_a_family() {
        let out = plane()
            .call("query", &json!({ "commands": "everruns agents list" }))
            .await;
        let value: Value = serde_json::from_str(&out).expect("json");
        assert_eq!(value["total"], 2, "{out}");
        assert_eq!(value["data"][0]["name"], "Triage");
    }

    /// The whole point of the fake: an absent flag is rejected by the real
    /// parser, so an offline case measures the same failure production has.
    #[tokio::test]
    async fn an_absent_flag_is_rejected_by_the_real_parser() {
        let out = plane()
            .call(
                "query",
                &json!({ "commands": "everruns skills list --limit 20" }),
            )
            .await;
        assert!(out.contains("unexpected argument"), "{out}");
        assert!(out.contains("--limit"), "{out}");
    }

    #[tokio::test]
    async fn a_flat_wire_name_still_works() {
        let out = plane()
            .call("query", &json!({ "commands": "list_harnesses" }))
            .await;
        assert!(out.contains("Generic"), "{out}");
    }

    #[tokio::test]
    async fn query_refuses_a_mutation_and_execute_runs_it() {
        let refused = plane()
            .call(
                "query",
                &json!({ "commands": "everruns agents create --name Bot --system-prompt Hi" }),
            )
            .await;
        assert!(refused.contains("use execute"), "{refused}");

        let plane = plane();
        let created = plane
            .call(
                "execute",
                &json!({ "commands": "everruns agents create --name Bot --system-prompt Hi" }),
            )
            .await;
        assert!(created.contains("\"name\":\"Bot\""), "{created}");
        // And it persists for the rest of the turn, so a follow-up read works.
        let listed = plane
            .call("query", &json!({ "commands": "everruns agents list" }))
            .await;
        assert!(listed.contains("Bot"), "{listed}");
    }

    #[tokio::test]
    async fn a_bare_word_reaches_the_positional_field() {
        let out = plane()
            .call(
                "query",
                &json!({ "commands": "everruns agents get agent_01triage" }),
            )
            .await;
        assert!(out.contains("Triage"), "{out}");
    }

    #[tokio::test]
    async fn discover_finds_an_operation_and_carries_its_spelling() {
        let out = plane()
            .call("discover", &json!({ "query": "list_harnesses" }))
            .await;
        assert!(out.contains("list_harnesses"), "{out}");
        assert!(
            out.contains("input_schema"),
            "an exact lookup carries schemas: {out}"
        );
    }

    /// A broad search must not expand every schema, which is the rule the real
    /// surface follows to keep discovery affordable.
    #[tokio::test]
    async fn a_broad_search_omits_schemas() {
        let out = plane().call("discover", &json!({ "query": "agent" })).await;
        assert!(!out.contains("input_schema"), "{out}");
    }

    /// The help is the tree's, not a local restatement: a hand-rolled list of
    /// nouns without descriptions is a different surface from the one that
    /// ships, and in v2 `--help` is nearly the whole discovery story.
    #[tokio::test]
    async fn help_is_the_shipped_rendering() {
        let root = plane()
            .call("query", &json!({ "commands": "everruns --help" }))
            .await;
        assert!(
            root.contains("Agent definitions, versions, and their configuration."),
            "node descriptions travel with the help: {root}"
        );
        let node = plane()
            .call("query", &json!({ "commands": "everruns agents --help" }))
            .await;
        assert!(
            node.contains("Run `everruns agents <command> --help` for flags."),
            "{node}"
        );
        let unknown = plane()
            .call(
                "query",
                &json!({ "commands": "everruns agents frobnicate" }),
            )
            .await;
        assert!(
            unknown.contains("unknown command `frobnicate`"),
            "{unknown}"
        );
    }

    #[tokio::test]
    async fn help_lists_the_nouns_and_a_nodes_verbs() {
        let root = plane()
            .call("query", &json!({ "commands": "everruns --help" }))
            .await;
        assert!(root.contains("agents"), "{root}");
        let node = plane()
            .call("query", &json!({ "commands": "everruns agents" }))
            .await;
        assert!(node.contains("versions") && node.contains("list"), "{node}");
    }

    #[tokio::test]
    async fn a_pipeline_runs_its_command_and_ignores_the_filter() {
        let out = plane()
            .call(
                "query",
                &json!({ "commands": "everruns agents list | jq -r '.data[].id'" }),
            )
            .await;
        assert!(out.contains("agent_01triage"), "{out}");
    }

    #[tokio::test]
    async fn several_statements_run_in_order() {
        let plane = plane();
        let out = plane.call(
            "execute",
            &json!({ "commands": "everruns agents create --name A --system-prompt X\neverruns agents list" }),
        ).await;
        assert!(out.contains("\"name\":\"A\""), "{out}");
        assert!(out.matches("Triage").count() >= 1, "{out}");
    }
}

/// The v2 arm: one `bash` tool over a real interpreter.
///
/// These are the cases v1's statement splitter got wrong or could not express
/// at all. They are the reason the A/B is worth running: if the shell arm can
/// loop, pipe and keep a file, its failures are the model's and the contract's
/// rather than the fake's.
#[cfg(test)]
mod shell_tests {
    use super::*;

    fn shell() -> FakeControlPlane {
        FakeControlPlane::new(Harness::PlatformChatV2)
    }

    #[test]
    fn a_help_probe_is_not_a_mutation() {
        assert!(script_mutates("everruns agents create --name Bot"));
        assert!(!script_mutates("everruns agents create --help"));
        assert!(!script_mutates("everruns agents list | jq '.data'"));
        // A script that reads and then writes is still a write.
        assert!(script_mutates(
            "everruns agents create --help\neveruns agents list\neverruns agents update --id a"
        ));
    }

    #[tokio::test]
    async fn v2_ships_one_tool_and_it_is_bash() {
        let tools = tool_definitions(Harness::PlatformChatV2);
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t["function"]["name"].as_str())
            .collect();
        assert_eq!(names, vec!["bash"]);
        // And the prompt it ships with does not send the model after tools it
        // no longer has.
        let prompt = system_prompt(Harness::PlatformChatV2);
        assert!(!prompt.contains("`discover`"), "{prompt}");
        assert!(!prompt.contains("`query`"), "{prompt}");
        assert!(!prompt.contains("`execute`"), "{prompt}");
    }

    #[tokio::test]
    async fn everruns_is_a_builtin_in_the_shell() {
        let out = shell()
            .call("bash", &json!({ "commands": "everruns agents list" }))
            .await;
        assert!(out.contains("Triage"), "{out}");
    }

    /// v1's splitter truncated at the first `|` and returned the command's own
    /// JSON. Here the filter runs.
    #[tokio::test]
    async fn a_pipeline_runs_its_filter() {
        let out = shell()
            .call(
                "bash",
                &json!({ "commands": "everruns agents list | jq -r '.data[].name'" }),
            )
            .await;
        assert!(out.contains("Triage"), "{out}");
        assert!(
            !out.contains("\"id\""),
            "jq projected, not passed through: {out}"
        );
    }

    /// The defect that made v1's arm unfair: `for … do … done` was split into
    /// words and the loop body never ran as a loop.
    #[tokio::test]
    async fn a_loop_runs_as_a_loop() {
        let out = shell()
            .call(
                "bash",
                &json!({ "commands": "for name in Alpha Beta; do everruns agents create --name \"$name\" --system-prompt Hi > /dev/null; done\neveruns_count=$(everruns agents list | jq '.total')\necho \"total=$everuns_count\"" }),
            )
            .await;
        assert!(
            out.contains("total=4"),
            "two seeded plus two created: {out}"
        );
    }

    /// Redirection and a scratch file, which v1 has no filesystem for.
    #[tokio::test]
    async fn output_survives_in_the_workspace_between_calls() {
        let plane = shell();
        plane
            .call(
                "bash",
                &json!({ "commands": "everruns harnesses list > /workspace/harnesses.json" }),
            )
            .await;
        let out = plane
            .call(
                "bash",
                &json!({ "commands": "jq -r '.data[].name' /workspace/harnesses.json" }),
            )
            .await;
        assert!(out.contains("Generic"), "{out}");
    }

    #[tokio::test]
    async fn help_reaches_the_shell_and_a_bad_flag_fails_nonzero() {
        let out = shell()
            .call("bash", &json!({ "commands": "everruns --help" }))
            .await;
        assert!(out.contains("agents"), "{out}");

        let bad = shell()
            .call(
                "bash",
                &json!({ "commands": "everruns skills list --limit 20" }),
            )
            .await;
        assert!(bad.contains("unexpected argument"), "{bad}");
        assert!(
            bad.contains("[exit "),
            "a parse error is a failed command: {bad}"
        );
    }

    /// The shell surface has no read-only gate: a mutation runs, because the
    /// split into `query` and `execute` was a property of having two tools.
    #[tokio::test]
    async fn a_mutation_runs_without_a_second_tool() {
        let out = shell()
            .call(
                "bash",
                &json!({ "commands": "everruns agents create --name Bot --system-prompt Hi" }),
            )
            .await;
        assert!(out.contains("\"name\":\"Bot\""), "{out}");
    }

    #[tokio::test]
    async fn the_docs_mount_is_readable() {
        let out = shell()
            .call(
                "bash",
                &json!({ "commands": "grep -r Generic /workspace/docs" }),
            )
            .await;
        assert!(out.contains("Generic"), "{out}");
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
    #[tokio::test]
    async fn a_nested_collection_is_its_own_family() {
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

    #[tokio::test]
    async fn a_created_version_is_listed_by_its_own_command() {
        let plane = FakeControlPlane::new(Harness::PlatformChat);
        plane.call(
            "execute",
            &json!({ "commands": "create_agent_version --agent_id agent_01triage --req {\"summary\":\"before\"}" }),
        ).await;
        let listed = plane
            .call(
                "query",
                &json!({ "commands": "list_agent_versions --agent_id agent_01triage" }),
            )
            .await;
        assert!(listed.contains("before"), "{listed}");
        // And the agents family is untouched.
        let agents = plane
            .call("query", &json!({ "commands": "everruns agents list" }))
            .await;
        let value: Value = serde_json::from_str(&agents).expect("json");
        assert_eq!(value["total"], 2, "{agents}");
    }
}
