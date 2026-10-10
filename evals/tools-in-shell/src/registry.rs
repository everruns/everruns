//! A small fake tool registry: three MCP-style sources and two top-level tools.
//!
//! Small on purpose. The question here is whether a model finds and calls a
//! tool it cannot see, through `tools --help` and `tools search`, not how
//! search ranks among hundreds; that is the success bar's larger slice.
//! The set still carries the traps the cases need: two similar search tools on
//! one source, a tool whose name is not the one a person would guess
//! (`find-contact`, not `get-contact`), and a tool with a strict input schema.
//!
//! Every handler records the call in a [`CallLog`], because a call made from a
//! script is not its own tool event (Tools in Shell D5): the log is the only
//! record of which tools ran and with what input.

use std::sync::{Arc, Mutex};

use everruns::FunctionTool;
use serde_json::{Value, json};

/// The sources the registry groups tools under, as `tools <source> <tool>`.
pub const SOURCES: &[&str] = &["crm", "tickets", "calendar"];

/// Calls the fake tools received, in order: `{tool, input}`.
#[derive(Clone, Default)]
pub struct CallLog(Arc<Mutex<Vec<Value>>>);

impl CallLog {
    pub fn record(&self, tool: &str, input: &Value) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(json!({ "tool": tool, "input": input }));
    }

    pub fn calls(&self) -> Vec<Value> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

struct Spec {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
    answer: fn(&Value) -> Value,
}

const SPECS: &[Spec] = &[
    Spec {
        name: "mcp_crm__find_contact",
        description: "Find contacts by name, email or company. Returns each match with its contact id.",
        schema: || {
            json!({
                "type": "object",
                "properties": {"query": {"type": "string", "description": "Name, email or company."}},
                "required": ["query"],
                "additionalProperties": false
            })
        },
        answer: find_contact,
    },
    Spec {
        name: "mcp_crm__list_deals",
        description: "List deals, optionally for one contact and one stage.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "contact_id": {"type": "string"},
                    "stage": {"type": "string", "enum": ["open", "won", "lost"]}
                },
                "additionalProperties": false
            })
        },
        answer: list_deals,
    },
    Spec {
        name: "mcp_crm__update_deal",
        description: "Change a deal's stage or amount.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "stage": {"type": "string", "enum": ["open", "won", "lost"]},
                    "amount": {"type": "number"}
                },
                "required": ["id"],
                "additionalProperties": false
            })
        },
        answer: |input| json!({"updated": input}),
    },
    Spec {
        name: "mcp_tickets__search_tickets",
        description: "Search customer support tickets by text and status.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string"},
                    "status": {"type": "string", "enum": ["open", "closed"]}
                },
                "required": ["text"],
                "additionalProperties": false
            })
        },
        answer: |_| {
            json!([
                {"id": "TCK-0981", "title": "Customer locked out after password change", "status": "closed"}
            ])
        },
    },
    Spec {
        name: "mcp_tickets__search_articles",
        description: "Search published help-center articles by text.",
        schema: || {
            json!({
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
                "additionalProperties": false
            })
        },
        answer: |_| {
            json!([
                {"id": "KB-12", "title": "Reset your password", "url": "https://help.example.com/kb/12"},
                {"id": "KB-31", "title": "Recover an account without two-factor codes", "url": "https://help.example.com/kb/31"}
            ])
        },
    },
    Spec {
        name: "mcp_tickets__create_ticket",
        description: "Open a support ticket.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "priority": {"type": "string", "enum": ["low", "normal", "high"]},
                    "labels": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["title", "priority"],
                "additionalProperties": false
            })
        },
        answer: |input| {
            let mut ticket = json!({"id": "TCK-1042", "status": "open"});
            if let (Some(ticket), Some(input)) = (ticket.as_object_mut(), input.as_object()) {
                for (key, value) in input {
                    ticket.insert(key.clone(), value.clone());
                }
            }
            ticket
        },
    },
    Spec {
        name: "mcp_calendar__list_events",
        description: "List the events on one day of the team calendar.",
        schema: || {
            json!({
                "type": "object",
                "properties": {"date": {"type": "string", "description": "YYYY-MM-DD"}},
                "required": ["date"],
                "additionalProperties": false
            })
        },
        answer: |input| match input.get("date").and_then(Value::as_str) {
            Some("2026-10-12") => json!([
                {"time": "10:00", "title": "Quarterly planning", "room": "Orion"},
                {"time": "15:30", "title": "Vendor call: Analytical Engines Ltd"}
            ]),
            _ => json!([]),
        },
    },
    Spec {
        name: "weather_forecast",
        description: "Daily weather forecast for a city, up to seven days ahead.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "city": {"type": "string"},
                    "days": {"type": "integer", "minimum": 1, "maximum": 7}
                },
                "required": ["city"],
                "additionalProperties": false
            })
        },
        answer: |input| {
            let days = input
                .get("days")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .clamp(1, 7) as usize;
            let pattern = [("sunny", 24), ("cloudy", 21), ("rain", 18)];
            json!({
                "city": input.get("city").cloned().unwrap_or(Value::Null),
                "days": (0..days)
                    .map(|day| {
                        let (sky, high) = pattern[day % pattern.len()];
                        json!({"day": day + 1, "sky": sky, "high_c": high})
                    })
                    .collect::<Vec<_>>()
            })
        },
    },
    Spec {
        name: "currency_convert",
        description: "Convert an amount between two currencies at today's rate.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "amount": {"type": "number"},
                    "from": {"type": "string"},
                    "to": {"type": "string"}
                },
                "required": ["amount", "from", "to"],
                "additionalProperties": false
            })
        },
        answer: |input| json!({"converted": input.get("amount").cloned().unwrap_or(json!(0)), "rate": 1.0}),
    },
];

const CONTACTS: &[(&str, &str, &str, &str)] = &[
    (
        "ct_ada",
        "Ada Lovelace",
        "ada@example.com",
        "Analytical Engines Ltd",
    ),
    (
        "ct_grace",
        "Grace Hopper",
        "grace@example.com",
        "Compilers Inc",
    ),
];

/// `(id, contact, stage, amount)`.
const DEALS: &[(&str, &str, &str, u64)] = &[
    ("deal_1", "ct_ada", "won", 40_000),
    ("deal_2", "ct_ada", "open", 5_000),
    ("deal_3", "ct_grace", "open", 12_000),
    ("deal_4", "ct_grace", "open", 8_500),
    ("deal_5", "ct_grace", "lost", 3_000),
];

fn find_contact(input: &Value) -> Value {
    let query = input
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let matches: Vec<Value> = CONTACTS
        .iter()
        .filter(|(_, name, email, company)| {
            let haystack = format!("{name} {email} {company}").to_lowercase();
            !words.is_empty() && words.iter().all(|word| haystack.contains(word))
        })
        .map(|(id, name, email, company)| {
            json!({"id": id, "name": name, "email": email, "company": company})
        })
        .collect();
    json!(matches)
}

fn list_deals(input: &Value) -> Value {
    let contact = input.get("contact_id").and_then(Value::as_str);
    let stage = input.get("stage").and_then(Value::as_str);
    json!(
        DEALS
            .iter()
            .filter(|(_, owner, deal_stage, _)| {
                contact.is_none_or(|c| c == *owner) && stage.is_none_or(|s| s == *deal_stage)
            })
            .map(|(id, owner, deal_stage, amount)| {
                json!({"id": id, "contact_id": owner, "stage": deal_stage, "amount_usd": amount})
            })
            .collect::<Vec<_>>()
    )
}

/// The registry as Framework tools, each recording into `log`.
pub fn tools(log: &CallLog) -> Vec<FunctionTool> {
    SPECS
        .iter()
        .map(|spec| {
            let log = log.clone();
            let name = spec.name;
            let answer = spec.answer;
            FunctionTool::new(spec.name, spec.description, (spec.schema)(), move |input| {
                let log = log.clone();
                async move {
                    log.record(name, &input);
                    Ok::<_, String>(answer(&input))
                }
            })
        })
        .collect()
}

/// Every tool name, for dataset validation.
#[cfg(test)]
pub fn names() -> Vec<&'static str> {
    SPECS.iter().map(|spec| spec.name).collect()
}
