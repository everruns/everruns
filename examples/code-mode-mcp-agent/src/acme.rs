//! The Acme MCP server: a synthetic company API with 1,000 tools.
//!
//! 25 services, 8 entities each, 5 verbs per entity. It speaks
//! newline-delimited JSON-RPC over stdio, pages `tools/list` 100 tools at a
//! time, and labels every tool with MCP read-only or destructive hints. Only
//! the billing and CRM list/get tools carry data; every other tool answers with
//! an empty but plausible payload, and every write is refused.

use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use serde_json::{Value, json};

/// The name the agent knows this server by.
pub const SERVER: &str = "acme";

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

// ---------------------------------------------------------------------------
// The MCP server: newline-delimited JSON-RPC over stdio, paginated tools/list.
// ---------------------------------------------------------------------------

/// Serve over this process's stdin and stdout until stdin closes.
pub fn serve() -> io::Result<()> {
    serve_on(io::stdin().lock(), io::BufWriter::new(io::stdout().lock()))
}

fn serve_on(input: impl BufRead, mut output: impl Write) -> io::Result<()> {
    let tools = catalog();
    let data = Dataset::generate();
    for line in input.lines() {
        let Ok(request) = serde_json::from_str::<Value>(&line?) else {
            continue;
        };
        let Some(id) = request.get("id").cloned() else {
            continue; // notifications need no reply
        };
        let params = &request["params"];
        let result = match request["method"].as_str().unwrap_or_default() {
            "initialize" => json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "acme-internal-apis", "version": "1.0.0"},
            }),
            "tools/list" => {
                const PAGE: usize = 100;
                let start = params["cursor"]
                    .as_str()
                    .and_then(|cursor| cursor.parse().ok())
                    .unwrap_or(0usize)
                    .min(tools.len());
                let end = (start + PAGE).min(tools.len());
                let next = (end < tools.len()).then(|| end.to_string());
                json!({"tools": &tools[start..end], "nextCursor": next})
            }
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default();
                let reply = data.call(name, &params["arguments"]);
                let is_error = reply.get("error").is_some();
                json!({
                    "content": [{"type": "text", "text": reply.to_string()}],
                    "isError": is_error,
                })
            }
            _ => {
                let error = json!({"code": -32601, "message": "Method not found"});
                writeln!(
                    output,
                    "{}",
                    json!({"jsonrpc": "2.0", "id": id, "error": error})
                )?;
                output.flush()?;
                continue;
            }
        };
        writeln!(
            output,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "result": result})
        )?;
        output.flush()?;
    }
    Ok(())
}

/// Every tool definition the server publishes.
pub fn catalog() -> Vec<Value> {
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
            format!(
                "List {noun} in {service} ({area}). Returns {{\"items\": [...], \"next_cursor\": string or null, \"total\": number}}."
            ),
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
pub fn top_overdue_eu_customers(n: usize) -> Vec<(String, u64)> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(requests: &[Value]) -> Vec<Value> {
        let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
        let mut output = Vec::new();
        serve_on(input.as_bytes(), &mut output).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn catalog_has_a_thousand_unique_labelled_tools() {
        let tools = catalog();
        assert_eq!(tools.len(), 1_000);
        let names: std::collections::HashSet<_> = tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), 1_000);
        assert!(names.contains("billing_list_invoices"));
        assert!(names.contains("crm_get_customer"));
        assert!(
            tools
                .iter()
                .all(|tool| tool["annotations"]["readOnlyHint"].is_boolean())
        );
    }

    #[test]
    fn tools_list_pages_through_the_whole_catalog() {
        let mut cursor = Value::Null;
        let mut seen = 0;
        for page in 1.. {
            let replies = exchange(&[
                json!({"jsonrpc": "2.0", "id": page, "method": "tools/list", "params": {"cursor": cursor}}),
            ]);
            seen += replies[0]["result"]["tools"].as_array().unwrap().len();
            cursor = replies[0]["result"]["nextCursor"].clone();
            if cursor.is_null() {
                break;
            }
        }
        assert_eq!(seen, 1_000);
    }

    #[test]
    fn notifications_get_no_reply_and_unknown_methods_an_error() {
        let replies = exchange(&[
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list"}),
        ]);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["error"]["code"], -32601);
    }

    #[test]
    fn invoice_filters_and_writes_behave_like_the_answer_assumes() {
        let replies = exchange(&[
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "billing_list_invoices", "arguments": {"status": "overdue", "limit": 500}}}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "crm_delete_customer", "arguments": {"id": "cus_1000"}}}),
        ]);
        let text = replies[0]["result"]["content"][0]["text"].as_str().unwrap();
        let page: Value = serde_json::from_str(text).unwrap();
        let invoices = page["items"].as_array().unwrap();
        assert!(!invoices.is_empty());
        assert!(
            invoices
                .iter()
                .all(|invoice| invoice["status"] == "overdue")
        );
        assert_eq!(replies[1]["result"]["isError"], true);
    }

    #[test]
    fn expected_answer_is_stable_and_ranked() {
        let top = top_overdue_eu_customers(3);
        assert_eq!(top.len(), 3);
        assert!(top[0].1 >= top[1].1 && top[1].1 >= top[2].1);
        assert_eq!(top, top_overdue_eu_customers(3));
    }
}
