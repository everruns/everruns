//! Public documentation classification is emitted by the API, never inferred by a renderer.

use serde_json::{Value, json};
use utoipa::{
    Modify,
    openapi::{OpenApi, RefOr, Schema, schema::Ref},
};

pub(super) struct DocumentationMetadata;

const GROUPS: &[(&str, &str, &str, &[&str])] = &[
    (
        "agent-api",
        "Agent API",
        "Call one agent: start sessions, send messages, stream results and respond to questions or tool approvals.",
        &["agent-execution"],
    ),
    (
        "agents-authoring",
        "Agents & authoring",
        "Configure agents, harnesses, capabilities, skills and channels. Manage keys, triggers and recorded changes.",
        &[
            "agents",
            "agent-channels",
            "harnesses",
            "capabilities",
            "skills",
            "agent-scripts",
            "agent-triggers",
            "agent-examples",
            "harness-examples",
            "history",
            "apps",
            "voice",
        ],
    ),
    (
        "sessions-workspaces",
        "Sessions & workspaces",
        "Manage execution sessions and their files, tasks, storage, databases and managed sandboxes.",
        &[
            "sessions",
            "messages",
            "events",
            "Files",
            "images",
            "session-resources",
            "session-sandbox",
            "session-databases",
            "session-storage",
            "session-tasks",
            "Session Git",
            "workspace",
            "workspace-filesystem",
        ],
    ),
    (
        "knowledge-memory",
        "Knowledge & memory",
        "Give agents persistent memory and versioned file collections. Manage sources, files and optional search indexes.",
        &["memory", "knowledge"],
    ),
    (
        "models-integrations",
        "Models & integrations",
        "Connect model providers, discover models and configure MCP servers, search providers and plugins.",
        &[
            "providers",
            "models",
            "mcp-servers",
            "mcp_servers",
            "web-search",
            "plugins",
        ],
    ),
    (
        "identity-access",
        "Identity & access",
        "Manage organizations, members, end users, invitations and connected identities.",
        &[
            "users",
            "virtual-users",
            "credentials",
            "Organizations",
            "organization invitations",
        ],
    ),
    (
        "evaluation-reporting",
        "Evaluation & reporting",
        "Inspect agent quality, health, observers, semantic reports and notifications.",
        &[
            "evals",
            "observers",
            "health-issues",
            "reporting",
            "notifications",
        ],
    ),
    (
        "operations-billing",
        "Operations & billing",
        "Operate durable execution, schedules, sandbox infrastructure, budgets, payments and audit logs.",
        &[
            "durable",
            "monitoring",
            "tracing",
            "system",
            "durable-schedules",
            "sandbox-templates",
            "sandboxes",
            "budgets",
            "payments",
            "task-webhooks",
            "audit_logs",
            "commands",
        ],
    ),
];

impl Modify for DocumentationMetadata {
    fn modify(&self, openapi: &mut OpenApi) {
        let mut value = serde_json::to_value(&*openapi).expect("OpenAPI serializes");
        apply(&mut value);
        // Utoipa's generated schemas are not all deserialization round trips.
        // Update only owned metadata and preserve the original typed schemas.
        openapi
            .extensions
            .get_or_insert_with(Default::default)
            .insert("x-tagGroups".to_owned(), value["x-tagGroups"].clone());
        openapi.tags =
            Some(serde_json::from_value(value["tags"].clone()).expect("Documentation tags"));
        for (path, item) in &mut openapi.paths.paths {
            for (method, slot) in [
                ("get", &mut item.get),
                ("post", &mut item.post),
                ("put", &mut item.put),
                ("patch", &mut item.patch),
                ("delete", &mut item.delete),
                ("head", &mut item.head),
                ("options", &mut item.options),
                ("trace", &mut item.trace),
            ] {
                let Some(operation) = slot else {
                    continue;
                };
                let docs = &value["paths"][path][method];
                operation.summary = docs["summary"].as_str().map(str::to_owned);
                operation.description = docs["description"].as_str().map(str::to_owned);
                operation.tags =
                    Some(serde_json::from_value(docs["tags"].clone()).expect("Operation tags"));
                if docs.get("security").is_some() {
                    operation.security = Some(
                        serde_json::from_value(docs["security"].clone())
                            .expect("Security requirements"),
                    );
                }
                let extensions = operation.extensions.get_or_insert_with(Default::default);
                for name in [
                    "x-docs",
                    "x-superseded-by",
                    "x-deprecation",
                    "x-codeSamples",
                ] {
                    if let Some(extension) = docs.get(name) {
                        extensions.insert(name.to_owned(), extension.clone());
                    }
                }
                if method == "post"
                    && matches!(
                        path.as_str(),
                        "/v1/channels/{channel_id}/sessions" | "/v1/e/{channel_id}/sessions"
                    )
                {
                    let RefOr::T(body) = operation.request_body.as_mut().expect("Session body")
                    else {
                        panic!("Platform session request body must be inline");
                    };
                    body.required = Some(utoipa::openapi::Required::False);
                    let RefOr::T(content) = body
                        .content
                        .get_mut("application/json")
                        .expect("Session JSON body")
                    else {
                        panic!("Platform session JSON content must be inline");
                    };
                    content.schema = Some(RefOr::Ref(Ref::from_schema_name(
                        "CreatePlatformAgentSessionRequest",
                    )));
                    content.example = Some(json!({"title":"Order 42"}));
                }
            }
        }
        let components = openapi.components.as_mut().expect("API components");
        let mut platform = components.schemas["CreateAgentSessionRequest"].clone();
        if let RefOr::T(Schema::Object(object)) = &mut platform {
            object.properties.remove("metadata");
            object.description = Some("Start an idle session on an Agent API channel. The body may be empty. Send a message to begin a turn.".to_owned());
            if let Some(RefOr::T(Schema::Object(title))) = object.properties.get_mut("title") {
                title.max_length = Some(200);
            }
        }
        components
            .schemas
            .insert("CreatePlatformAgentSessionRequest".to_owned(), platform);
        components.security_schemes.insert(
            "AgentCredential".to_owned(),
            serde_json::from_value(
                value["components"]["securitySchemes"]["AgentCredential"].clone(),
            )
            .expect("Agent bearer scheme"),
        );
    }
}

fn apply(spec: &mut Value) {
    spec["x-tagGroups"] = json!(
        GROUPS
            .iter()
            .map(|(slug, name, description, tags)| json!({
                "name": name, "tags": tags, "x-slug": slug, "x-description": description,
            }))
            .collect::<Vec<_>>()
    );

    let paths = spec["paths"].as_object_mut().expect("OpenAPI paths");
    let canonical: Vec<_> = paths
        .iter()
        .flat_map(|(path, item)| {
            item.as_object().into_iter().flat_map(move |item| {
                item.iter().filter_map(move |(method, op)| {
                    if !is_method(method) || op["deprecated"] == true {
                        return None;
                    }
                    Some((
                        path.clone(),
                        method.clone(),
                        op["operationId"].as_str()?.to_owned(),
                    ))
                })
            })
        })
        .collect();

    for (path, item) in paths.iter_mut() {
        for (method, operation) in item.as_object_mut().expect("Path item") {
            if !is_method(method) {
                continue;
            }
            let id = operation["operationId"]
                .as_str()
                .expect("Operation ID")
                .to_owned();
            // Runtime auth is part of the caller API, despite sharing identity infrastructure.
            if path == "/v1/channels/{channel_id}/runtime-auth"
                || path == "/v1/e/{endpoint_id}/runtime-auth"
            {
                operation["tags"] = json!(["agent-execution"]);
            }
            let tag = operation["tags"][0].as_str().expect("Operation tag");
            let (group, _, _, _) = GROUPS
                .iter()
                .find(|(_, _, _, tags)| tags.contains(&tag))
                .unwrap_or_else(|| panic!("Classify OpenAPI tag {tag} before publishing"));
            let internal = tag == "durable"
                || path.starts_with("/v1/reports/admin/")
                || path == "/v1/reports/projector/run";
            let experimental = tag == "payments";
            operation["x-docs"] = json!({
                "group": group,
                // No current API has an announced stable compatibility guarantee.
                "support": if experimental { "experimental" } else { "preview" },
                "audience": if internal { "internal" } else { "public" },
                "availability": if experimental { "Requires machine_payments; disabled by default" }
                    else if tag == "monitoring" { "Requires organization adoption of the monitoring feature flag" }
                    else if tag == "evals" { "Requires the evals feature flag" }
                    else { "Everruns Cloud and self-hosted Platform" },
            });
            if let Some(summary) = agent_summary(&id) {
                operation["summary"] = json!(summary);
            } else if operation.get("summary").is_none() {
                operation["summary"] = json!(humanize(&id));
            }
            // A bare `METHOD /path` summary says nothing the page does not
            // already show; the operation ID reads better as a title.
            if operation["summary"].as_str().is_some_and(|summary| {
                summary.split_once(' ').is_some_and(|(method, rest)| {
                    is_method(&method.to_lowercase())
                        && rest.starts_with('/')
                        && !rest.contains(' ')
                })
            }) {
                operation["summary"] = json!(humanize(&id));
            }
            if let Some(summary) = operation["summary"].as_str() {
                let (title, full) = split_summary(summary);
                if title != summary {
                    if full.trim_end_matches('.') != title && operation.get("description").is_none()
                    {
                        operation["description"] = json!(full);
                    }
                    operation["summary"] = json!(title);
                }
            }

            if operation["deprecated"] == true {
                let replacement_path = path
                    .replace("/v1/e/", "/v1/channels/")
                    .replace("/endpoints", "/channels")
                    .replace("{endpoint_id}", "{channel_id}");
                if let Some((_, _, replacement)) = canonical
                    .iter()
                    .find(|(p, m, _)| p == &replacement_path && m == method)
                {
                    operation["x-superseded-by"] = json!(replacement);
                    operation["x-deprecation"] = json!({"migration": "Use the canonical channel route. The legacy alias remains available; no removal date has been announced."});
                } else {
                    operation["x-deprecation"] = json!({"migration": "Read-only archival App records. Use agent channels for new integrations; there is no drop-in replacement."});
                }
            }
            if group == &"agent-api" {
                let public_read = method == "get"
                    && (path.ends_with("/chat/config")
                        || path.ends_with("/chat/shared/{token}")
                        || path == "/v1/work-chat/hosts/{label}");
                operation["security"] = if public_read {
                    json!([])
                } else {
                    json!([{"AgentCredential": []}])
                };
            }
        }
    }

    // The Framework's shared create-session type also accepts metadata. The Platform rejects it.
    let schema = json!({"type":"object", "description":"Start an idle session on an Agent API channel. The body may be empty. Send a message to begin a turn.",
        "properties":{"title":{"type":["string","null"],"maxLength":200,"description":"Optional session title.","example":"Order 42"}}});
    spec["components"]["schemas"]["CreatePlatformAgentSessionRequest"] = schema;
    for path in [
        "/v1/channels/{channel_id}/sessions",
        "/v1/e/{channel_id}/sessions",
    ] {
        spec["paths"][path]["post"]["requestBody"]["required"] = json!(false);
        spec["paths"][path]["post"]["requestBody"]["content"]["application/json"]["schema"] =
            json!({"$ref":"#/components/schemas/CreatePlatformAgentSessionRequest"});
        spec["paths"][path]["post"]["requestBody"]["content"]["application/json"]["example"] =
            json!({"title":"Order 42"});
    }
    spec["components"]["securitySchemes"]["AgentCredential"] = json!({"type":"http","scheme":"bearer",
        "description":"Agent key (evr_ak_…), channel runtime token, or a token accepted by the channel's configured identity provider. Personal access tokens require org_members. Runtime-auth accepts an agent key with End-User or a configured identity-provider token; runtime tokens cannot renew themselves."});

    spec["paths"]["/v1/channels/{channel_id}/sessions"]["post"]["x-codeSamples"] = json!([
        {"lang":"bash","label":"cURL","source":"curl --request POST \\\n  \"$EVERRUNS_AGENT_URL/sessions\" \\\n  --header \"Authorization: Bearer $EVERRUNS_AGENT_KEY\" \\\n  --header \"Content-Type: application/json\" \\\n  --data '{\"title\": \"Order 42\"}'"},
        {"lang":"typescript","label":"TypeScript","source":"import { AgentClient } from \"@everruns/sdk\";\n\nconst agent = new AgentClient();\nconst session = await agent.createSession({\n  title: \"Order 42\",\n  idempotencyKey: \"order-42-start\",\n});"},
        {"lang":"python","label":"Python","source":"from everruns_sdk import AgentClient\n\nasync with AgentClient() as agent:\n    session = await agent.create_session(\n        \"Order 42\",\n        idempotency_key=\"order-42-start\",\n    )"},
    ]);

    let descriptions = spec["tags"].as_array().cloned().unwrap_or_default();
    let used: std::collections::BTreeSet<_> = spec["paths"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|item| {
            item.as_object()
                .unwrap()
                .iter()
                .filter(|(m, _)| is_method(m))
                .flat_map(|(_, op)| {
                    op["tags"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(Value::as_str)
                })
        })
        .map(str::to_owned)
        .collect();
    spec["tags"] = json!(
        GROUPS
            .iter()
            .flat_map(|(_, _, _, tags)| tags.iter())
            .filter(|name| used.contains(**name))
            .map(|name| {
                let mut tag = descriptions
                    .iter()
                    .find(|t| t["name"] == *name)
                    .cloned()
                    .unwrap_or_else(|| json!({"name":name}));
                tag["x-displayName"] = json!(match *name {
                    "agent-execution" => "Agent API".to_owned(),
                    "mcp_servers" | "mcp-servers" => "MCP servers".to_owned(),
                    _ => humanize(name),
                });
                tag
            })
            .collect::<Vec<_>>()
    );
}

fn is_method(method: &str) -> bool {
    matches!(
        method,
        "get" | "post" | "put" | "patch" | "delete" | "head" | "options" | "trace"
    )
}

/// Longest summary that still makes a page title under 70 characters with
/// the docs site's ` | Everruns` suffix. The summary titles the endpoint's
/// reference page; the rest belongs in the description.
#[cfg(test)]
const MAX_SUMMARY: usize = 58;

/// Splits a summary into its title (no `METHOD /path - ` prefix, first
/// sentence, no final period) and the full text without the prefix.
fn split_summary(summary: &str) -> (String, String) {
    let full = summary
        .split_once(' ')
        .filter(|(method, rest)| is_method(&method.to_lowercase()) && rest.starts_with('/'))
        .and_then(|(_, rest)| {
            rest.split_once(" - ")
                .or_else(|| rest.split_once(" \u{2014} "))
                .map(|(_, text)| text)
        })
        .unwrap_or(summary)
        .trim();
    let end = full
        .match_indices(". ")
        .map(|(i, _)| i)
        .find(|&i| !["e.g", "i.e", "vs"].iter().any(|a| full[..i].ends_with(a)))
        .unwrap_or(full.len());
    let title = full[..end].trim_end_matches('.');
    let mut chars = title.chars();
    let title = chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default();
    (title, full.to_owned())
}

fn humanize(id: &str) -> String {
    let text = id.replace(['_', '-'], " ");
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn agent_summary(id: &str) -> Option<&'static str> {
    Some(match id.trim_end_matches("_legacy") {
        "agent_api_get_card" => "Get agent card",
        "agent_api_create_session" => "Start a session",
        "agent_api_list_sessions" => "List sessions",
        "agent_api_get_session" => "Get a session",
        "agent_api_send_message" => "Send a message",
        "agent_api_list_events" => "Read events",
        "agent_api_stream_events" => "Stream events",
        "agent_api_answer_questions" => "Answer questions",
        "agent_api_submit_tool_approvals" => "Approve tool calls",
        "agent_api_cancel" => "Cancel a turn",
        "agent_api_add_reaction" => "Add a reaction",
        "agent_api_remove_reaction" => "Remove a reaction",
        "agent_api_list_reactions" => "List reactions",
        "exchange" | "exchange_legacy" => "Create a runtime token",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openapi::ApiDoc;
    use utoipa::OpenApi as _;

    #[test]
    fn summaries_fit_a_page_title() {
        assert_eq!(
            split_summary("PUT /v1/agents/{agent_id} - Create or update agent (upsert)"),
            (
                "Create or update agent (upsert)".to_owned(),
                "Create or update agent (upsert)".to_owned()
            )
        );
        assert_eq!(
            split_summary("Start a run, e.g. a retry. Returns the run."),
            (
                "Start a run, e.g. a retry".to_owned(),
                "Start a run, e.g. a retry. Returns the run.".to_owned()
            )
        );
        let spec = serde_json::to_value(ApiDoc::openapi()).unwrap();
        assert_eq!(
            spec["paths"]["/v1/harnesses/{harness_id}"]["get"]["summary"],
            "Get harness"
        );
        let long: Vec<String> = spec["paths"]
            .as_object()
            .unwrap()
            .values()
            .flat_map(|item| item.as_object().unwrap().iter())
            .filter(|(method, _)| is_method(method))
            .filter_map(|(_, op)| {
                let summary = op["summary"].as_str()?;
                (summary.chars().count() > MAX_SUMMARY)
                    .then(|| format!("{}: {summary}", op["operationId"].as_str().unwrap_or("?")))
            })
            .collect();
        assert!(
            long.is_empty(),
            "OpenAPI summaries title the API reference pages, keep them to {MAX_SUMMARY} \
             characters (`summary = \"..\"` on #[command], or a shorter first doc line):\n{}",
            long.join("\n")
        );
    }

    #[test]
    fn every_operation_has_public_classification_and_legacy_successors_resolve() {
        let spec = serde_json::to_value(ApiDoc::openapi()).unwrap();
        assert_eq!(spec["x-tagGroups"][0]["name"], "Agent API");
        let operations: Vec<_> = spec["paths"]
            .as_object()
            .unwrap()
            .values()
            .flat_map(|item| {
                item.as_object()
                    .unwrap()
                    .iter()
                    .filter(|(m, _)| is_method(m))
                    .map(|(_, op)| op)
            })
            .collect();
        let ids: std::collections::BTreeSet<_> = operations
            .iter()
            .map(|op| op["operationId"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids.len(),
            operations.len(),
            "Every endpoint needs its own operation ID and documentation URL"
        );
        for (path, method, id) in [
            ("/v1/harness-examples", "get", "list_examples"),
            ("/v1/sessions/{session_id}/tasks", "get", "list_tasks"),
            ("/v1/files/{file_id}", "get", "get_file"),
            ("/v1/files/{file_id}", "delete", "delete_file"),
            (
                "/v1/orgs/{org}/organization-connections",
                "get",
                "list_connections",
            ),
            (
                "/v1/orgs/{org}/organization-connections/{connection_id}",
                "delete",
                "delete_connection",
            ),
            (
                "/v1/orgs/{org}/organization-connections/{connection_id}/verify",
                "post",
                "verify_connection",
            ),
            (
                "/v1/virtual-users/{identity_id}/sessions",
                "get",
                "list_sessions",
            ),
        ] {
            assert_eq!(
                spec["paths"][path][method]["operationId"], id,
                "Keep the previously published URL's endpoint: {path}"
            );
        }
        for operation in &operations {
            assert!(operation["x-docs"]["group"].is_string());
            assert!(matches!(
                operation["x-docs"]["support"].as_str(),
                Some("preview" | "experimental")
            ));
            assert!(operation["summary"].is_string());
            if let Some(id) = operation["x-superseded-by"].as_str() {
                assert_eq!(operation["deprecated"], true);
                assert!(
                    operations
                        .iter()
                        .any(|op| op["operationId"] == id && op["deprecated"] != true)
                );
            }
        }
        let request = &spec["paths"]["/v1/channels/{channel_id}/sessions"]["post"];
        assert_eq!(request["requestBody"]["required"], false);
        assert_eq!(request["security"][0]["AgentCredential"], json!([]));
        let platform = &spec["components"]["schemas"]["CreatePlatformAgentSessionRequest"];
        assert!(platform["properties"].get("metadata").is_none());
        assert!(
            spec["components"]["schemas"]["CreateAgentSessionRequest"]["properties"]
                .get("metadata")
                .is_some()
        );
    }
}
