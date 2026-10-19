//! A fake tool registry: 26 tools across CRM, billing, calendar, tickets and
//! inventory, every one deferred behind `tool_search`.
//!
//! The schemas are the point of the study, so they are deliberately not
//! trivial: required fields, enums whose values a person would not guess
//! (`fra2` for Frankfurt, `p2` for "P2"), nested objects, arrays of objects,
//! integer ranges, money in cents, and `additionalProperties: false`
//! everywhere. Some names sit close together (`create_invoice` /
//! `create_quote`, `create_event` / `create_recurring_event`,
//! `find_contact` / `find_company`) so a model that calls from memory rather
//! than from the schema it was shown has room to go wrong.
//!
//! Every handler validates the arguments it received against its own schema
//! with `jsonschema` and records the call in a [`CallLog`]. In practice the
//! engine's tool registry already checks arguments against the tool's
//! registered (full) schema before the handler runs, in both arms, so this is
//! a second gate that should never fire; the scorers grade every attempt from
//! the tool events (`subject::attempts`), and this log cross-checks which calls
//! actually ran. A refused call returns a model-visible tool error naming each
//! problem.

use std::sync::{Arc, Mutex};

use everruns::{FunctionTool, ToolResponse};
use serde_json::{Value, json};

/// Calls the fake tools received, in order: `{tool, input, valid, errors}`.
#[derive(Clone, Default)]
pub struct CallLog(Arc<Mutex<Vec<Value>>>);

impl CallLog {
    pub fn record(&self, tool: &str, input: &Value, errors: &[String]) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(json!({
                "tool": tool,
                "input": input,
                "valid": errors.is_empty(),
                "errors": errors,
            }));
    }

    pub fn calls(&self) -> Vec<Value> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

pub struct Spec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: fn() -> Value,
    pub answer: fn(&Value) -> Value,
}

const DATE: &str = r"^\d{4}-\d{2}-\d{2}$";
const DATETIME: &str = r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$";
const EMAIL: &str = r"^[^@\s]+@[^@\s]+\.[a-z]{2,}$";

fn warehouse() -> Value {
    json!({
        "type": "string",
        "enum": ["ams1", "fra2", "iad1"],
        "description": "Warehouse code: ams1 = Amsterdam, fra2 = Frankfurt, iad1 = Virginia (US)."
    })
}

fn sku() -> Value {
    json!({"type": "string", "pattern": "^SKU-\\d{4}$", "description": "Product SKU, e.g. SKU-1002."})
}

pub const SPECS: &[Spec] = &[
    // ---------------------------------------------------------------- CRM
    Spec {
        name: "crm_find_contact",
        description: "Find people in the CRM by name, email or company. Returns matches with their contact ids.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "minLength": 2},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 20}
                },
                "required": ["query"],
                "additionalProperties": false
            })
        },
        answer: find_contact,
    },
    Spec {
        name: "crm_find_company",
        description: "Find companies (accounts) in the CRM by name or domain.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "minLength": 2},
                    "domain": {"type": "string"}
                },
                "anyOf": [{"required": ["name"]}, {"required": ["domain"]}],
                "additionalProperties": false
            })
        },
        answer: |_| json!([{"id": "co_compilers", "name": "Compilers Inc", "domain": "compilers.example"}]),
    },
    Spec {
        name: "crm_create_contact",
        description: "Create a contact in the CRM.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "first_name": {"type": "string", "minLength": 1},
                    "last_name": {"type": "string", "minLength": 1},
                    "email": {"type": "string", "pattern": EMAIL},
                    "company_id": {"type": "string", "pattern": "^co_[a-z0-9_]+$"},
                    "lifecycle_stage": {"type": "string", "enum": ["lead", "prospect", "customer", "churned"]},
                    "tags": {"type": "array", "items": {"type": "string"}, "maxItems": 5, "uniqueItems": true}
                },
                "required": ["first_name", "last_name", "email", "lifecycle_stage"],
                "additionalProperties": false
            })
        },
        answer: |input| json!({"id": "ct_new_1", "created": input}),
    },
    Spec {
        name: "crm_update_contact",
        description: "Change fields on an existing CRM contact.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "contact_id": {"type": "string", "pattern": "^ct_[a-z0-9_]+$"},
                    "changes": {
                        "type": "object",
                        "properties": {
                            "email": {"type": "string", "pattern": EMAIL},
                            "phone": {"type": "string", "pattern": "^\\+\\d{7,15}$", "description": "E.164, e.g. +441632960961."},
                            "lifecycle_stage": {"type": "string", "enum": ["lead", "prospect", "customer", "churned"]}
                        },
                        "minProperties": 1,
                        "additionalProperties": false
                    }
                },
                "required": ["contact_id", "changes"],
                "additionalProperties": false
            })
        },
        answer: |input| json!({"updated": input}),
    },
    Spec {
        name: "crm_log_activity",
        description: "Record a call, email, meeting or note against a CRM contact.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "contact_id": {"type": "string", "pattern": "^ct_[a-z0-9_]+$"},
                    "activity": {
                        "type": "object",
                        "properties": {
                            "type": {"type": "string", "enum": ["call", "email", "meeting", "note"]},
                            "summary": {"type": "string", "minLength": 3, "maxLength": 500},
                            "duration_minutes": {"type": "integer", "minimum": 1, "maximum": 480},
                            "occurred_on": {"type": "string", "pattern": DATE, "description": "YYYY-MM-DD"}
                        },
                        "required": ["type", "summary", "occurred_on"],
                        "additionalProperties": false
                    }
                },
                "required": ["contact_id", "activity"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"activity_id": "act_5521", "status": "logged"}),
    },
    Spec {
        name: "crm_list_deals",
        description: "List CRM deals, optionally for one contact and one stage.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "contact_id": {"type": "string", "pattern": "^ct_[a-z0-9_]+$"},
                    "stage": {"type": "string", "enum": ["open", "won", "lost"]}
                },
                "additionalProperties": false
            })
        },
        answer: |_| json!([{"id": "deal_3", "contact_id": "ct_grace", "stage": "open", "amount_usd": 12000}]),
    },
    // ------------------------------------------------------------ billing
    Spec {
        name: "billing_get_customer",
        description: "Look up a billing customer by email and return their customer id.",
        schema: || {
            json!({
                "type": "object",
                "properties": {"email": {"type": "string", "pattern": EMAIL}},
                "required": ["email"],
                "additionalProperties": false
            })
        },
        answer: |input| json!({"customer_id": "cus_4471", "email": input.get("email")}),
    },
    Spec {
        name: "billing_create_invoice",
        description: "Issue an invoice to a billing customer. Amounts are integer cents.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "customer_id": {"type": "string", "pattern": "^cus_\\d+$"},
                    "currency": {"type": "string", "enum": ["USD", "EUR", "GBP"]},
                    "line_items": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 20,
                        "items": {
                            "type": "object",
                            "properties": {
                                "sku": sku(),
                                "quantity": {"type": "integer", "minimum": 1, "maximum": 1000},
                                "unit_price_cents": {"type": "integer", "minimum": 0}
                            },
                            "required": ["sku", "quantity", "unit_price_cents"],
                            "additionalProperties": false
                        }
                    },
                    "due_in_days": {"type": "integer", "minimum": 0, "maximum": 90},
                    "memo": {"type": "string", "maxLength": 200}
                },
                "required": ["customer_id", "currency", "line_items", "due_in_days"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"invoice_id": "inv_9310", "status": "open"}),
    },
    Spec {
        name: "billing_create_quote",
        description: "Prepare a price quote (not an invoice) for a billing customer.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "customer_id": {"type": "string", "pattern": "^cus_\\d+$"},
                    "currency": {"type": "string", "enum": ["USD", "EUR", "GBP"]},
                    "items": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {"sku": sku(), "quantity": {"type": "integer", "minimum": 1}},
                            "required": ["sku", "quantity"],
                            "additionalProperties": false
                        }
                    },
                    "valid_until": {"type": "string", "pattern": DATE}
                },
                "required": ["customer_id", "currency", "items", "valid_until"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"quote_id": "q_118", "status": "draft"}),
    },
    Spec {
        name: "billing_issue_refund",
        description: "Refund part or all of a paid invoice. Amounts are integer cents.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "invoice_id": {"type": "string", "pattern": "^inv_\\d+$"},
                    "amount_cents": {"type": "integer", "minimum": 1},
                    "reason": {"type": "string", "enum": ["duplicate", "fraudulent", "requested_by_customer", "product_defect"]},
                    "notify_customer": {"type": "boolean"}
                },
                "required": ["invoice_id", "amount_cents", "reason", "notify_customer"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"refund_id": "re_7781", "status": "succeeded"}),
    },
    Spec {
        name: "billing_list_invoices",
        description: "List a billing customer's invoices.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "customer_id": {"type": "string", "pattern": "^cus_\\d+$"},
                    "status": {"type": "string", "enum": ["draft", "open", "paid", "void"]},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 50}
                },
                "required": ["customer_id"],
                "additionalProperties": false
            })
        },
        answer: |_| json!([{"invoice_id": "inv_9001", "status": "paid", "total_cents": 5100}]),
    },
    Spec {
        name: "billing_apply_discount",
        description: "Apply a discount to an open invoice.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "invoice_id": {"type": "string", "pattern": "^inv_\\d+$"},
                    "discount": {
                        "type": "object",
                        "properties": {
                            "kind": {"type": "string", "enum": ["percent", "fixed_cents"]},
                            "value": {"type": "integer", "minimum": 1}
                        },
                        "required": ["kind", "value"],
                        "additionalProperties": false
                    }
                },
                "required": ["invoice_id", "discount"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"status": "applied"}),
    },
    // ----------------------------------------------------------- calendar
    Spec {
        name: "calendar_list_events",
        description: "List the events on one day of the team calendar.",
        schema: || {
            json!({
                "type": "object",
                "properties": {"date": {"type": "string", "pattern": DATE}},
                "required": ["date"],
                "additionalProperties": false
            })
        },
        answer: |_| json!([]),
    },
    Spec {
        name: "calendar_find_free_slots",
        description: "Find times when all the given people are free.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "attendees": {"type": "array", "minItems": 1, "items": {"type": "string", "pattern": EMAIL}},
                    "duration_minutes": {"type": "integer", "enum": [15, 30, 45, 60, 90]},
                    "window": {
                        "type": "object",
                        "properties": {
                            "from": {"type": "string", "pattern": DATE},
                            "to": {"type": "string", "pattern": DATE}
                        },
                        "required": ["from", "to"],
                        "additionalProperties": false
                    }
                },
                "required": ["attendees", "duration_minutes", "window"],
                "additionalProperties": false
            })
        },
        answer: |_| json!([{"start": "2026-10-20T14:00"}]),
    },
    Spec {
        name: "calendar_create_event",
        description: "Create a one-off calendar event.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "minLength": 1},
                    "start": {"type": "string", "pattern": DATETIME, "description": "Local time, YYYY-MM-DDTHH:MM."},
                    "duration_minutes": {"type": "integer", "minimum": 5, "maximum": 480},
                    "attendees": {
                        "type": "array",
                        "minItems": 1,
                        "items": {
                            "type": "object",
                            "properties": {
                                "email": {"type": "string", "pattern": EMAIL},
                                "optional": {"type": "boolean"}
                            },
                            "required": ["email", "optional"],
                            "additionalProperties": false
                        }
                    },
                    "location": {
                        "type": "object",
                        "properties": {
                            "kind": {"type": "string", "enum": ["room", "video", "phone"]},
                            "value": {"type": "string", "minLength": 1}
                        },
                        "required": ["kind", "value"],
                        "additionalProperties": false
                    },
                    "reminders_minutes": {
                        "type": "array",
                        "maxItems": 3,
                        "items": {"type": "integer", "minimum": 0, "maximum": 1440}
                    }
                },
                "required": ["title", "start", "duration_minutes", "attendees"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"event_id": "ev_3320", "status": "confirmed"}),
    },
    Spec {
        name: "calendar_create_recurring_event",
        description: "Create a calendar event that repeats on a schedule.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "minLength": 1},
                    "first_start": {"type": "string", "pattern": DATETIME, "description": "First occurrence, local time, YYYY-MM-DDTHH:MM."},
                    "duration_minutes": {"type": "integer", "minimum": 5, "maximum": 480},
                    "recurrence": {
                        "type": "object",
                        "properties": {
                            "frequency": {"type": "string", "enum": ["daily", "weekly", "monthly"]},
                            "interval": {"type": "integer", "minimum": 1, "maximum": 12},
                            "by_weekday": {
                                "type": "array",
                                "minItems": 1,
                                "uniqueItems": true,
                                "items": {"type": "string", "enum": ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]}
                            },
                            "occurrences": {"type": "integer", "minimum": 1, "maximum": 52}
                        },
                        "required": ["frequency", "interval", "occurrences"],
                        "additionalProperties": false
                    },
                    "attendees": {"type": "array", "minItems": 1, "items": {"type": "string", "pattern": EMAIL}}
                },
                "required": ["title", "first_start", "duration_minutes", "recurrence", "attendees"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"series_id": "evs_118", "status": "confirmed"}),
    },
    Spec {
        name: "calendar_cancel_event",
        description: "Cancel a calendar event.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "event_id": {"type": "string", "pattern": "^ev_\\d+$"},
                    "notify_attendees": {"type": "boolean"},
                    "reason": {"type": "string"}
                },
                "required": ["event_id", "notify_attendees"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"status": "cancelled"}),
    },
    // ------------------------------------------------------------ tickets
    Spec {
        name: "tickets_search_tickets",
        description: "Search customer support tickets by text and status.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string", "minLength": 2},
                    "status": {"type": "string", "enum": ["open", "pending", "solved", "closed"]}
                },
                "required": ["text"],
                "additionalProperties": false
            })
        },
        answer: search_tickets,
    },
    Spec {
        name: "tickets_create_ticket",
        description: "Open a customer support ticket.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "minLength": 3, "maxLength": 120},
                    "priority": {"type": "string", "enum": ["p1", "p2", "p3", "p4"], "description": "p1 is the most urgent."},
                    "category": {"type": "string", "enum": ["billing", "technical", "account", "other"]},
                    "customer": {
                        "type": "object",
                        "properties": {
                            "email": {"type": "string", "pattern": EMAIL},
                            "plan": {"type": "string", "enum": ["free", "pro", "enterprise"]}
                        },
                        "required": ["email", "plan"],
                        "additionalProperties": false
                    },
                    "labels": {"type": "array", "items": {"type": "string", "pattern": "^[a-z0-9-]+$"}, "maxItems": 5}
                },
                "required": ["title", "priority", "category", "customer"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"ticket_id": "TCK-6001", "status": "open"}),
    },
    Spec {
        name: "tickets_escalate_ticket",
        description: "Escalate a support ticket to another team.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ticket_id": {"type": "string", "pattern": "^TCK-\\d+$"},
                    "level": {"type": "integer", "minimum": 1, "maximum": 3},
                    "team": {"type": "string", "enum": ["tier2", "engineering", "billing_ops"]},
                    "note": {"type": "string", "minLength": 5}
                },
                "required": ["ticket_id", "level", "team", "note"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"status": "escalated"}),
    },
    Spec {
        name: "tickets_merge_tickets",
        description: "Merge duplicate support tickets into one primary ticket.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "primary_id": {"type": "string", "pattern": "^TCK-\\d+$"},
                    "duplicate_ids": {"type": "array", "minItems": 1, "items": {"type": "string", "pattern": "^TCK-\\d+$"}}
                },
                "required": ["primary_id", "duplicate_ids"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"status": "merged"}),
    },
    Spec {
        name: "tickets_add_comment",
        description: "Add a comment to a support ticket.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "ticket_id": {"type": "string", "pattern": "^TCK-\\d+$"},
                    "body": {"type": "string", "minLength": 1},
                    "visibility": {"type": "string", "enum": ["public", "internal"]}
                },
                "required": ["ticket_id", "body", "visibility"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"comment_id": "cm_44"}),
    },
    // ---------------------------------------------------------- inventory
    Spec {
        name: "inventory_search_products",
        description: "Search the product catalogue by name and return SKUs.",
        schema: || {
            json!({
                "type": "object",
                "properties": {"query": {"type": "string", "minLength": 2}},
                "required": ["query"],
                "additionalProperties": false
            })
        },
        answer: search_products,
    },
    Spec {
        name: "inventory_check_stock",
        description: "Check how many units of a SKU are in stock at a warehouse.",
        schema: || {
            json!({
                "type": "object",
                "properties": {"sku": sku(), "warehouse": warehouse()},
                "required": ["sku"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"on_hand": 120, "reserved": 15}),
    },
    Spec {
        name: "inventory_reserve_stock",
        description: "Reserve units of a SKU at one warehouse for a customer order.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "sku": sku(),
                    "warehouse": warehouse(),
                    "quantity": {"type": "integer", "minimum": 1, "maximum": 500},
                    "order_ref": {"type": "string", "pattern": "^ORD-\\d+$"}
                },
                "required": ["sku", "warehouse", "quantity", "order_ref"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"reservation_id": "res_812", "status": "held"}),
    },
    Spec {
        name: "inventory_transfer_stock",
        description: "Move units of a SKU from one warehouse to another.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "sku": sku(),
                    "from_warehouse": warehouse(),
                    "to_warehouse": warehouse(),
                    "quantity": {"type": "integer", "minimum": 1, "maximum": 5000},
                    "priority": {"type": "string", "enum": ["standard", "express"]}
                },
                "required": ["sku", "from_warehouse", "to_warehouse", "quantity"],
                "additionalProperties": false
            })
        },
        answer: |_| json!({"transfer_id": "tr_95", "status": "scheduled"}),
    },
];

fn query_words(input: &Value, key: &str) -> Vec<String> {
    input
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn find_contact(input: &Value) -> Value {
    const CONTACTS: &[(&str, &str, &str, &str)] = &[
        (
            "ct_ada",
            "Ada Lovelace",
            "ada@analytical.example",
            "Analytical Engines Ltd",
        ),
        (
            "ct_grace",
            "Grace Hopper",
            "grace@compilers.example",
            "Compilers Inc",
        ),
        (
            "ct_grace_k",
            "Grace Kelly",
            "gkelly@monaco.example",
            "Monaco Films",
        ),
    ];
    let words = query_words(input, "query");
    json!(
        CONTACTS
            .iter()
            .filter(|(_, name, email, company)| {
                let hay = format!("{name} {email} {company}").to_lowercase();
                !words.is_empty() && words.iter().all(|w| hay.contains(w.as_str()))
            })
            .map(|(id, name, email, company)| {
                json!({"contact_id": id, "name": name, "email": email, "company": company})
            })
            .collect::<Vec<_>>()
    )
}

fn search_tickets(input: &Value) -> Value {
    const TICKETS: &[(&str, &str, &str)] = &[
        ("TCK-5120", "SSO login loop after Okta update", "open"),
        ("TCK-4988", "SSO login loop on Safari", "solved"),
        ("TCK-5133", "Invoice PDF shows wrong VAT", "open"),
    ];
    let words = query_words(input, "text");
    let status = input.get("status").and_then(Value::as_str);
    json!(
        TICKETS
            .iter()
            .filter(|(_, title, st)| {
                let hay = title.to_lowercase();
                status.is_none_or(|s| s == *st)
                    && words
                        .iter()
                        .any(|w| w.len() > 2 && hay.contains(w.as_str()))
            })
            .map(|(id, title, st)| json!({"ticket_id": id, "title": title, "status": st}))
            .collect::<Vec<_>>()
    )
}

fn search_products(input: &Value) -> Value {
    const PRODUCTS: &[(&str, &str)] = &[
        ("SKU-3307", "Orion standing desk"),
        ("SKU-3308", "Orion desk lamp"),
        ("SKU-1002", "Vega office chair"),
    ];
    let words = query_words(input, "query");
    json!(
        PRODUCTS
            .iter()
            .filter(|(_, name)| {
                let hay = name.to_lowercase();
                !words.is_empty() && words.iter().all(|w| hay.contains(w.as_str()))
            })
            .map(|(sku, name)| json!({"sku": sku, "name": name}))
            .collect::<Vec<_>>()
    )
}

/// Validate `input` against `validator`, one readable line per problem.
fn problems(validator: &jsonschema::Validator, input: &Value) -> Vec<String> {
    validator
        .iter_errors(input)
        .map(|error| {
            let path = error.instance_path().to_string();
            let at = if path.is_empty() {
                "(root)".to_string()
            } else {
                path
            };
            format!("{at}: {error}")
        })
        .collect()
}

/// The registry as Framework tools, each validating and recording into `log`.
pub fn tools(log: &CallLog) -> Vec<FunctionTool> {
    SPECS
        .iter()
        .map(|spec| {
            let log = log.clone();
            let name = spec.name;
            let answer = spec.answer;
            let schema = (spec.schema)();
            let validator = Arc::new(
                jsonschema::validator_for(&schema)
                    .unwrap_or_else(|e| panic!("{name}: schema does not compile: {e}")),
            );
            FunctionTool::new(name, spec.description, schema, move |input: Value| {
                let log = log.clone();
                let validator = validator.clone();
                async move {
                    let errors = problems(&validator, &input);
                    log.record(name, &input, &errors);
                    if errors.is_empty() {
                        Ok::<_, String>(ToolResponse::json(answer(&input)))
                    } else {
                        Ok(ToolResponse::error(format!(
                            "invalid_arguments for {name}: {}",
                            errors.join("; ")
                        )))
                    }
                }
            })
        })
        .collect()
}

/// The schema a tool declares, for the offline tests.
#[cfg(test)]
pub fn schema_of(name: &str) -> Option<Value> {
    SPECS.iter().find(|s| s.name == name).map(|s| (s.schema)())
}

pub fn names() -> Vec<&'static str> {
    SPECS.iter().map(|spec| spec.name).collect()
}
