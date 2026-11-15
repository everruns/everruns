//! Code mode over a huge MCP server: 1,000 tools, one `bash` tool in the prompt.
//!
//! Attaching a large MCP server natively puts every tool schema into every
//! model request. This example attaches a synthetic company API with 1,000
//! tools (25 services, 8 entities each, 5 verbs per entity) and turns on
//! `tools_in_shell` in CLI mode. The MCP tools leave the model's tool list;
//! the agent reaches them from the Bashkit sandbox through the `tools`
//! command instead: `tools search` to find operations, `--help` for one
//! signature, then a script that calls many tools, joins the results with
//! `jq`, and prints only the answer. The intermediate payloads never enter the
//! conversation.
//!
//! The MCP server is this same binary, re-executed with `--mcp-server`, so the
//! example needs nothing outside the repository. Its answer is checked against
//! the server's data.
//!
//! ```text
//! cargo run -p everruns --features openai,mcp-stdio,bashkit --example code_mode_mcp
//! ```
//!
//! Uses `gpt-6.1-sol` and `OPENAI_API_KEY`. Add the `openrouter` feature and
//! set `OPENROUTER_API_KEY` to run the same model through OpenRouter.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, BufRead, Write};

use everruns::{Agent, CapabilityRef, Engine, McpServer, OpenAI};
use serde_json::{Value, json};

const SERVER: &str = "acme";

/// Services with their own four entities; every service also has the four
/// shared ones below. 25 services x 8 entities x 5 verbs = 1,000 tools.
const SERVICES: [(&str, &str, [&str; 4]); 25] = [
    (
        "crm",
        "customer relationships",
        ["customers", "contacts", "deals", "accounts"],
    ),
    (
        "billing",
        "invoicing and payments",
        ["invoices", "payments", "refunds", "credit_notes"],
    ),
    (
        "inventory",
        "stock levels",
        ["items", "locations", "adjustments", "counts"],
    ),
    (
        "shipping",
        "outbound logistics",
        ["shipments", "carriers", "labels", "rates"],
    ),
    (
        "support",
        "customer support",
        ["tickets", "macros", "queues", "slas"],
    ),
    (
        "hr",
        "people records",
        ["employees", "teams", "positions", "leaves"],
    ),
    (
        "payroll",
        "salary runs",
        ["runs", "payslips", "deductions", "bonuses"],
    ),
    (
        "marketing",
        "campaigns",
        ["campaigns", "segments", "audiences", "assets"],
    ),
    (
        "analytics",
        "business metrics",
        ["dashboards", "metrics", "funnels", "cohorts"],
    ),
    (
        "procurement",
        "purchasing",
        ["purchase_orders", "suppliers", "quotes", "contracts"],
    ),
    (
        "legal",
        "legal matters",
        ["matters", "agreements", "clauses", "filings"],
    ),
    (
        "security",
        "security operations",
        ["incidents", "findings", "policies", "exceptions"],
    ),
    (
        "devops",
        "build and deploy",
        ["pipelines", "deployments", "environments", "releases"],
    ),
    (
        "monitoring",
        "service health",
        ["alerts", "monitors", "incidents", "silences"],
    ),
    (
        "storage",
        "file storage",
        ["buckets", "objects", "lifecycles", "quotas"],
    ),
    (
        "identity",
        "users and access",
        ["users", "groups", "roles", "sessions"],
    ),
    (
        "calendar",
        "scheduling",
        ["events", "rooms", "bookings", "availabilities"],
    ),
    (
        "docs",
        "internal documents",
        ["documents", "folders", "templates", "revisions"],
    ),
    (
        "chat",
        "team chat",
        ["channels", "messages", "threads", "reactions"],
    ),
    (
        "email",
        "outbound email",
        ["messages", "templates", "domains", "suppressions"],
    ),
    (
        "sales",
        "sales pipeline",
        ["leads", "opportunities", "forecasts", "territories"],
    ),
    (
        "finance",
        "general ledger",
        ["journals", "accounts", "budgets", "periods"],
    ),
    (
        "warehouse",
        "warehouse operations",
        ["bins", "picks", "putaways", "waves"],
    ),
    (
        "fleet",
        "vehicle fleet",
        ["vehicles", "drivers", "routes", "inspections"],
    ),
    (
        "facilities",
        "office facilities",
        ["sites", "assets", "work_orders", "visitors"],
    ),
];
const SHARED_ENTITIES: [&str; 4] = ["notes", "tags", "audit_events", "webhooks"];
const VERBS: [&str; 5] = ["list", "get", "create", "update", "delete"];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--mcp-server") {
        return Ok(serve_mcp()?);
    }
    tokio::runtime::Runtime::new()?.block_on(run_agent())
}

async fn run_agent() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = catalog();
    let native_bytes = serde_json::to_vec(&catalog)?.len();
    println!(
        "MCP server `{SERVER}`: {} tools, {} KiB of tool schemas if attached natively",
        catalog.len(),
        native_bytes / 1024
    );

    let server = McpServer::stdio(SERVER, std::env::current_exe()?.to_string_lossy())
        .arg("--mcp-server")
        .description("Acme Corp internal APIs: CRM, billing, inventory, HR and 21 more services");

    let (provider, model) = model()?;
    let agent = Agent::builder()
        .name("acme-analyst")
        .instructions(
            "You answer questions about Acme Corp from its internal APIs. The APIs are \
             available only inside the shell, through the `tools` command: find \
             operations with `tools search`, read one with `--help`, then do the work in \
             a script that calls the tools and filters the results with jq. Print only \
             what you need. Never guess figures. Reply briefly.",
        )
        .provider(provider)
        .model(model)
        .max_iterations(12)
        .capability("bashkit_shell")
        // CLI mode: eligible tools leave the native tool list and are callable
        // only through `tools` in the shell. `rate_unlabeled_tools: false`
        // keeps the decision service out; this server labels every tool with
        // MCP read-only/destructive hints.
        .capability(
            CapabilityRef::new("tools_in_shell")
                .config(json!({"mode": "cli", "rate_unlabeled_tools": false})),
        )
        .mcp_server(server)
        .build()?;

    let session = Engine::new().create(agent);
    let mut events = session.events();
    let question = "Which customers in the EU region have overdue invoices? Give the three \
                    with the largest total overdue amount, with each total in dollars.";
    println!("\nuser: {question}");
    let turn = session.run(question).await?;
    if !turn.success {
        return Err(format!("turn failed: {}", turn.error.unwrap_or_default()).into());
    }
    println!("\nagent: {}\n", turn.response.trim());

    // What the model saw and did, from the session's canonical events.
    let mut direct_calls: BTreeMap<String, usize> = BTreeMap::new();
    let mut nested_calls: BTreeMap<String, usize> = BTreeMap::new();
    let mut peak_tool_bytes = 0;
    let mut peak_input_tokens = 0;
    let mut model_tools: Vec<String> = Vec::new();
    while let Some(event) = events.try_recv()? {
        let canonical = event.canonical_json();
        let data = &canonical["data"];
        match event.event_type() {
            "tool.completed" => {
                let name = data["tool_name"].as_str().unwrap_or_default();
                *direct_calls.entry(name.to_owned()).or_default() += 1;
            }
            "tool.nested_call" if data["status"] == "completed" => {
                let name = data["tool_name"].as_str().unwrap_or_default();
                *nested_calls.entry(name.to_owned()).or_default() += 1;
            }
            "llm.generation" => {
                if let Some(tools) = data.pointer("/request_body/tools") {
                    peak_tool_bytes = peak_tool_bytes.max(tools.to_string().len());
                    model_tools = tools
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|tool| {
                            tool["name"]
                                .as_str()
                                .or_else(|| tool.pointer("/function/name").and_then(Value::as_str))
                                .map(str::to_owned)
                        })
                        .collect();
                }
                if let Some(usage) = data.pointer("/metadata/usage") {
                    let input = ["input_tokens", "cache_read_tokens", "cache_creation_tokens"]
                        .iter()
                        .filter_map(|key| usage[*key].as_u64())
                        .sum::<u64>();
                    peak_input_tokens = peak_input_tokens.max(input);
                }
            }
            _ => {}
        }
    }

    println!("tools in the model request: {model_tools:?}");
    if peak_tool_bytes > 0 {
        println!(
            "tool schemas per request: {} KiB (native would be {} KiB)",
            peak_tool_bytes.div_ceil(1024),
            native_bytes / 1024
        );
    }
    if peak_input_tokens > 0 {
        println!("largest prompt: {peak_input_tokens} input tokens");
    }
    println!("tools the model called directly: {direct_calls:?}");
    println!(
        "MCP tools called from scripts: {} calls {nested_calls:?}",
        nested_calls.values().sum::<usize>()
    );

    // The MCP tools never reached the model as native tools...
    if direct_calls.keys().any(|name| name.starts_with("mcp_")) {
        return Err("the model called an MCP tool directly instead of from the shell".into());
    }
    if nested_calls.is_empty() {
        return Err("no MCP tool was called from the shell".into());
    }
    // ...and the answer matches the server's data.
    let expected = top_overdue_eu_customers(3);
    let normalized = turn.response.replace(',', "");
    for (name, total) in &expected {
        if !turn.response.contains(name.as_str()) || !normalized.contains(&total.to_string()) {
            return Err(
                format!("answer is missing {name} (${total}); expected {expected:?}").into(),
            );
        }
    }
    println!("\nanswer verified against the server's data: {expected:?}");
    Ok(())
}

/// `gpt-6.1-sol` from OpenAI. Built with the `openrouter` feature and with
/// `OPENROUTER_API_KEY` set, the same model through OpenRouter instead.
fn model() -> Result<(everruns::Provider, &'static str), Box<dyn std::error::Error>> {
    #[cfg(feature = "openrouter")]
    if std::env::var_os("OPENROUTER_API_KEY").is_some() {
        let openrouter = everruns::providers::openrouter::OpenRouter::from_env()?;
        return Ok((openrouter.into(), "openai/gpt-6.1-sol"));
    }
    Ok((OpenAI::from_env()?.into(), "gpt-6.1-sol"))
}

// ---------------------------------------------------------------------------
// The MCP server: newline-delimited JSON-RPC over stdio, paginated tools/list.
// ---------------------------------------------------------------------------

fn serve_mcp() -> io::Result<()> {
    let tools = catalog();
    let data = Dataset::generate();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let Ok(request) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        let Some(id) = request.get("id").cloned() else {
            continue; // notifications need no reply
        };
        let params = &request["params"];
        let reply = match request["method"].as_str().unwrap_or_default() {
            "initialize" => json!({"result": {
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "acme-internal-apis", "version": "1.0.0"},
            }}),
            "tools/list" => {
                const PAGE: usize = 100;
                let start = params["cursor"]
                    .as_str()
                    .and_then(|cursor| cursor.parse().ok())
                    .unwrap_or(0usize)
                    .min(tools.len());
                let end = (start + PAGE).min(tools.len());
                let next = (end < tools.len()).then(|| end.to_string());
                json!({"result": {"tools": &tools[start..end], "nextCursor": next}})
            }
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                let output = data.call(name, &params["arguments"]);
                let is_error = output.get("error").is_some();
                json!({"result": {
                    "content": [{"type": "text", "text": output.to_string()}],
                    "isError": is_error,
                }})
            }
            _ => json!({"error": {"code": -32601, "message": "Method not found"}}),
        };
        let mut message = json!({"jsonrpc": "2.0", "id": id});
        message
            .as_object_mut()
            .expect("object")
            .extend(reply.as_object().expect("object").clone());
        writeln!(stdout, "{message}")?;
        stdout.flush()?;
    }
    Ok(())
}

/// Every tool definition the server publishes.
fn catalog() -> Vec<Value> {
    let mut tools = Vec::with_capacity(1_000);
    for (service, area, own) in SERVICES {
        for entity in own.iter().chain(SHARED_ENTITIES.iter()) {
            for verb in VERBS {
                tools.push(tool(service, area, entity, verb));
            }
        }
    }
    tools
}

fn tool(service: &str, area: &str, entity: &str, verb: &str) -> Value {
    let singular = singular(entity);
    let noun = entity.replace('_', " ");
    let one = singular.replace('_', " ");
    let id = json!({"type": "string", "description": format!("The {one} id.")});
    let fields = json!({"type": "object", "description": format!("Fields of the {one}."), "additionalProperties": true});
    let (description, properties, required): (String, Value, Vec<&str>) = match verb {
        "list" => (
            format!("List {noun} in {service} ({area}). Supports filters and paging."),
            list_filters(service, entity),
            vec![],
        ),
        "get" => (
            format!("Get one {one} from {service} ({area}) by id."),
            json!({"id": id}),
            vec!["id"],
        ),
        "create" => (
            format!("Create a {one} in {service} ({area})."),
            json!({"fields": fields}),
            vec!["fields"],
        ),
        "update" => (
            format!("Update a {one} in {service} ({area}). Only the given fields change."),
            json!({"id": id, "fields": fields}),
            vec!["id", "fields"],
        ),
        _ => (
            format!("Permanently delete a {one} from {service} ({area})."),
            json!({"id": id}),
            vec!["id"],
        ),
    };
    json!({
        "name": if verb == "list" {
            format!("{service}_list_{entity}")
        } else {
            format!("{service}_{verb}_{singular}")
        },
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required},
        "annotations": {
            "readOnlyHint": matches!(verb, "list" | "get"),
            "destructiveHint": verb == "delete",
        },
    })
}

fn singular(entity: &str) -> String {
    if let Some(stem) = entity.strip_suffix("ies") {
        format!("{stem}y")
    } else if let Some(stem) = entity.strip_suffix("uses") {
        format!("{stem}us")
    } else {
        entity.strip_suffix('s').unwrap_or(entity).to_owned()
    }
}

fn list_filters(service: &str, entity: &str) -> Value {
    let mut filters = json!({
        "limit": {"type": "integer", "description": "Page size, 1-500. Default 50."},
        "cursor": {"type": "string", "description": "Cursor from the previous page's next_cursor."},
        "updated_after": {"type": "string", "description": "ISO 8601 timestamp."},
    });
    match (service, entity) {
        ("billing", "invoices") => {
            filters["status"] =
                json!({"type": "string", "enum": ["open", "paid", "overdue", "void"]});
            filters["customer_id"] =
                json!({"type": "string", "description": "Only this customer's invoices."});
        }
        ("crm", "customers") => {
            filters["region"] = json!({"type": "string", "enum": ["EU", "NA", "APAC", "LATAM"]});
        }
        _ => {}
    }
    filters
}

// ---------------------------------------------------------------------------
// Deterministic data behind the tools that matter for the question. Every
// other tool answers with a generic, plausible payload.
// ---------------------------------------------------------------------------

struct Dataset {
    customers: Vec<Value>,
    invoices: Vec<Value>,
}

impl Dataset {
    fn generate() -> Self {
        const FIRST: [&str; 10] = [
            "Northwind",
            "Bluefin",
            "Granite",
            "Juniper",
            "Halcyon",
            "Ironbark",
            "Lumen",
            "Meridian",
            "Saffron",
            "Tidewater",
        ];
        const SECOND: [&str; 4] = ["Logistics", "Foods", "Robotics", "Textiles"];
        const REGIONS: [&str; 4] = ["EU", "NA", "APAC", "LATAM"];
        let mut seed: u64 = 0x5eed_c0de;
        let mut next = move |modulo: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % modulo
        };
        let customers: Vec<Value> = (0..40)
            .map(|i| {
                json!({
                    "id": format!("cus_{:04}", 1000 + i),
                    "name": format!("{} {}", FIRST[i % 10], SECOND[i / 10]),
                    "region": REGIONS[next(4) as usize],
                    "tier": (["standard", "plus", "enterprise"][next(3) as usize]),
                })
            })
            .collect();
        let invoices = (0..240)
            .map(|i| {
                let customer = &customers[next(customers.len() as u64) as usize];
                json!({
                    "id": format!("inv_{:05}", 20000 + i),
                    "customer_id": customer["id"],
                    "amount_usd": 200 + next(9_800),
                    "status": (["paid", "paid", "open", "overdue", "void"][next(5) as usize]),
                    "due_date": format!("2026-{:02}-{:02}", 1 + next(10), 1 + next(28)),
                })
            })
            .collect();
        Self {
            customers,
            invoices,
        }
    }

    fn call(&self, name: &str, args: &Value) -> Value {
        let text = |key: &str| args[key].as_str();
        match name {
            "billing_list_invoices" => {
                let items: Vec<&Value> = self
                    .invoices
                    .iter()
                    .filter(|inv| text("status").is_none_or(|s| inv["status"] == s))
                    .filter(|inv| text("customer_id").is_none_or(|c| inv["customer_id"] == c))
                    .collect();
                page(&items, args)
            }
            "crm_list_customers" => {
                let items: Vec<&Value> = self
                    .customers
                    .iter()
                    .filter(|c| text("region").is_none_or(|r| c["region"] == r))
                    .collect();
                page(&items, args)
            }
            "crm_get_customer" => match self
                .customers
                .iter()
                .find(|c| Some(c["id"].as_str().unwrap_or_default()) == text("id"))
            {
                Some(customer) => customer.clone(),
                None => json!({"error": "customer not found"}),
            },
            "billing_get_invoice" => match self
                .invoices
                .iter()
                .find(|i| Some(i["id"].as_str().unwrap_or_default()) == text("id"))
            {
                Some(invoice) => invoice.clone(),
                None => json!({"error": "invoice not found"}),
            },
            other if other.contains("_list_") => json!({"items": [], "next_cursor": null}),
            other if other.contains("_get_") => json!({"error": "not found"}),
            other if other.contains("_delete_") => {
                json!({"error": "permission denied: read-only demo"})
            }
            _ => json!({"error": "permission denied: read-only demo"}),
        }
    }
}

/// Page a filtered list the way every list tool does: `limit` (default 50,
/// max 500) and an offset `cursor`.
fn page(items: &[&Value], args: &Value) -> Value {
    let limit = args["limit"].as_u64().unwrap_or(50).clamp(1, 500) as usize;
    let start = args["cursor"]
        .as_str()
        .and_then(|cursor| cursor.parse().ok())
        .unwrap_or(0usize)
        .min(items.len());
    let end = (start + limit).min(items.len());
    let next = (end < items.len()).then(|| end.to_string());
    json!({"items": &items[start..end], "next_cursor": next, "total": items.len()})
}

/// The answer, computed straight from the data: EU customers by total
/// overdue amount, largest first.
fn top_overdue_eu_customers(n: usize) -> Vec<(String, u64)> {
    let data = Dataset::generate();
    let eu: HashMap<&str, &str> = data
        .customers
        .iter()
        .filter(|c| c["region"] == "EU")
        .map(|c| {
            (
                c["id"].as_str().unwrap_or_default(),
                c["name"].as_str().unwrap_or_default(),
            )
        })
        .collect();
    let mut totals: HashMap<&str, u64> = HashMap::new();
    for invoice in data.invoices.iter().filter(|i| i["status"] == "overdue") {
        if let Some(name) = eu.get(invoice["customer_id"].as_str().unwrap_or_default()) {
            *totals.entry(name).or_default() += invoice["amount_usd"].as_u64().unwrap_or(0);
        }
    }
    let mut ranked: Vec<(String, u64)> = totals
        .into_iter()
        .map(|(name, total)| (name.to_owned(), total))
        .collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.truncate(n);
    ranked
}
