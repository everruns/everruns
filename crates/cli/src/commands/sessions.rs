// Session management commands

use crate::output::{OutputFormat, print_field, print_table_header, print_table_row};
use anyhow::{Context, Result};
use clap::Subcommand;
use everruns_sdk::{CreateSessionRequest, Everruns};
use futures::StreamExt;
use std::collections::HashMap;

#[derive(Subcommand)]
pub enum SessionsCommand {
    /// Create a new session
    Create {
        /// Harness ID (e.g. harness_xxx). Omit to use org default.
        #[arg(long, short = 'H')]
        harness: Option<String>,

        /// Agent ID (optional, e.g. agent_xxx)
        #[arg(long, short)]
        agent: Option<String>,

        /// Session title
        #[arg(long)]
        title: Option<String>,

        /// Model ID override (e.g. mod_xxx)
        #[arg(long)]
        model: Option<String>,

        /// Session-scoped secret (repeatable, format: KEY=VALUE)
        #[arg(long = "secret", value_name = "KEY=VALUE")]
        secrets: Vec<String>,

        /// Budget limit (repeatable). Format: [CURRENCY:]LIMIT.
        /// Currency defaults to usd. Examples:
        ///   --budget-limit 10             ($10 USD)
        ///   --budget-limit usd:10         ($10 USD, explicit)
        ///   --budget-limit tokens:2000000 (2M token limit)
        /// Multiple limits stack — most restrictive wins.
        #[arg(long = "budget-limit", value_name = "[CURRENCY:]LIMIT")]
        budget_limits: Vec<String>,

        /// Budget soft limit — pauses before hard stop. Same format as --budget-limit.
        /// Must pair with a --budget-limit of the same currency.
        #[arg(long = "budget-soft-limit", value_name = "[CURRENCY:]LIMIT")]
        budget_soft_limits: Vec<String>,
    },

    /// List sessions
    List,

    /// Get session by ID
    Get {
        /// Session ID (e.g. ses_xxx)
        session: String,
    },

    /// Watch session events in real time
    Watch {
        /// Session ID (e.g. ses_xxx)
        session: String,
    },

    /// Export session messages as JSONL
    Export {
        /// Session ID (e.g. session_xxx)
        session: String,

        /// Output file path (defaults to stdout)
        #[arg(long, short)]
        output: Option<String>,
    },
}

pub async fn run(
    command: SessionsCommand,
    client: &Everruns,
    api_url: &str,
    api_key: &str,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    match command {
        SessionsCommand::Create {
            harness,
            agent,
            title,
            model,
            secrets,
            budget_limits,
            budget_soft_limits,
        } => {
            create(
                client,
                api_url,
                api_key,
                output,
                quiet,
                harness,
                agent,
                title,
                model,
                secrets,
                budget_limits,
                budget_soft_limits,
            )
            .await
        }
        SessionsCommand::List => list(client, output).await,
        SessionsCommand::Get { session } => get(client, output, session).await,
        SessionsCommand::Watch { session } => watch(client, output, session).await,
        SessionsCommand::Export {
            session,
            output: file_path,
        } => export(api_url, api_key, output, quiet, session, file_path).await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn create(
    client: &Everruns,
    api_url: &str,
    api_key: &str,
    output: OutputFormat,
    quiet: bool,
    harness_id: Option<String>,
    agent_id: Option<String>,
    title: Option<String>,
    model_id: Option<String>,
    raw_secrets: Vec<String>,
    raw_budget_limits: Vec<String>,
    raw_budget_soft_limits: Vec<String>,
) -> Result<()> {
    let secrets = parse_secrets(&raw_secrets)?;
    let budget_specs = parse_budget_limits(&raw_budget_limits, &raw_budget_soft_limits)?;

    let mut req = CreateSessionRequest::new();
    if let Some(h) = harness_id {
        req = req.harness_id(h);
    }
    if let Some(a) = agent_id {
        req = req.agent_id(a);
    }
    if let Some(t) = title {
        req = req.title(t);
    }
    if let Some(m) = model_id {
        req = req.model_id(m);
    }

    let session = client.sessions().create_with_options(req).await?;

    // Store secrets after session creation
    if !secrets.is_empty() {
        client
            .sessions()
            .set_secrets(&session.id, &secrets)
            .await
            .with_context(|| {
                format!(
                    "Failed to store {} secrets for session {}",
                    secrets.len(),
                    session.id
                )
            })?;
    }

    // Create budgets after session creation
    let mut created_budgets = Vec::new();
    for spec in &budget_specs {
        let budget = create_budget(
            api_url,
            api_key,
            &session.id,
            &spec.currency,
            spec.limit,
            spec.soft_limit,
        )
        .await
        .with_context(|| format!("Session {} created but budget creation failed", session.id))?;
        created_budgets.push(budget);
    }

    if output.is_text() {
        if quiet {
            println!("{}", session.id);
        } else {
            println!("Created session: {}", session.id);
            if let Some(agent) = &session.agent_id {
                print_field("Agent", agent);
            }
            let status = format!("{:?}", session.status).to_lowercase();
            print_field("Status", &status);
            if !secrets.is_empty() {
                print_field("Secrets", &format!("{} injected", secrets.len()));
            }
            for budget in &created_budgets {
                let budget_id = budget
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let currency = budget
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or("usd");
                let limit = budget.get("limit").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let soft = budget.get("soft_limit").and_then(|v| v.as_f64());
                let label = format_budget_amount(limit, currency);
                let detail = match soft {
                    Some(s) => format!(
                        "{}, soft {} ({})",
                        label,
                        format_budget_amount(s, currency),
                        budget_id
                    ),
                    None => format!("{} ({})", label, budget_id),
                };
                print_field("Budget", &detail);
            }
        }
    } else {
        let mut json = serde_json::to_value(&session)?;
        if !secrets.is_empty() {
            json["secrets_count"] = serde_json::json!(secrets.len());
        }
        if !created_budgets.is_empty() {
            json["budgets"] = serde_json::json!(created_budgets);
        }
        output.print_value(&json);
    }

    Ok(())
}

/// Create a budget for a session via the budgets API.
// TODO(sdk): Replace with client.budgets().create() when SDK adds budget support.
// Tracked: everruns/sdk#72, EVE-247
async fn create_budget(
    api_url: &str,
    api_key: &str,
    session_id: &str,
    currency: &str,
    limit: f64,
    soft_limit: Option<f64>,
) -> Result<serde_json::Value> {
    let url = format!("{}/v1/budgets", api_url.trim_end_matches('/'));
    let mut body = serde_json::json!({
        "subject_type": "session",
        "subject_id": session_id,
        "currency": currency,
        "limit": limit,
    });
    if let Some(soft) = soft_limit {
        body["soft_limit"] = serde_json::json!(soft);
    }

    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .context("Failed to create budget")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Budget creation failed ({}): {}", status, text);
    }

    resp.json::<serde_json::Value>()
        .await
        .context("Failed to parse budget response")
}

/// Parsed budget specification from `--budget-limit` and `--budget-soft-limit`.
struct BudgetSpec {
    limit: f64,
    currency: String,
    soft_limit: Option<f64>,
}

/// Parse a `[CURRENCY:]LIMIT` string into (currency, amount).
fn parse_currency_amount(s: &str) -> Result<(String, f64)> {
    if let Some((left, right)) = s.split_once(':') {
        // currency:amount
        let amount: f64 = right
            .parse()
            .with_context(|| format!("Invalid amount (expected number): {right}"))?;
        if left.is_empty() {
            anyhow::bail!("Currency cannot be empty in: {s}");
        }
        Ok((left.to_string(), amount))
    } else {
        // Just a number — default to usd
        let amount: f64 = s
            .parse()
            .with_context(|| format!("Invalid budget value (expected [CURRENCY:]LIMIT): {s}"))?;
        Ok(("usd".to_string(), amount))
    }
}

/// Build budget specs from `--budget-limit` and `--budget-soft-limit` flags.
fn parse_budget_limits(limits: &[String], soft_limits: &[String]) -> Result<Vec<BudgetSpec>> {
    // Parse hard limits
    let mut specs: Vec<BudgetSpec> = Vec::new();
    for entry in limits {
        let (currency, limit) = parse_currency_amount(entry)?;
        if limit <= 0.0 {
            anyhow::bail!("Budget limit must be positive: {entry}");
        }
        specs.push(BudgetSpec {
            limit,
            currency,
            soft_limit: None,
        });
    }

    // Parse and attach soft limits by currency
    for entry in soft_limits {
        let (currency, soft) = parse_currency_amount(entry)?;
        if soft <= 0.0 {
            anyhow::bail!("Budget soft limit must be positive: {entry}");
        }
        let matching = specs.iter_mut().find(|s| s.currency == currency);
        match matching {
            Some(spec) => {
                if soft >= spec.limit {
                    anyhow::bail!(
                        "Soft limit ({soft}) must be less than limit ({}) for currency '{currency}'",
                        spec.limit
                    );
                }
                spec.soft_limit = Some(soft);
            }
            None => {
                anyhow::bail!(
                    "--budget-soft-limit {entry} has no matching --budget-limit for currency '{currency}'"
                );
            }
        }
    }

    Ok(specs)
}

fn format_budget_amount(amount: f64, currency: &str) -> String {
    match currency {
        "usd" => format!("${:.2}", amount),
        "tokens" => format!("{} tokens", amount),
        "credits" => format!("{} credits", amount),
        other => format!("{} {}", amount, other),
    }
}

/// Parse "KEY=VALUE" strings into a HashMap.
fn parse_secrets(raw: &[String]) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for entry in raw {
        let (key, value) = entry
            .split_once('=')
            .with_context(|| format!("Invalid secret format (expected KEY=VALUE): {}", entry))?;
        if key.is_empty() {
            anyhow::bail!("Secret key cannot be empty: {}", entry);
        }
        if map.contains_key(key) {
            anyhow::bail!("Duplicate secret key: {}", key);
        }
        map.insert(key.to_string(), value.to_string());
    }
    Ok(map)
}

async fn list(client: &Everruns, output: OutputFormat) -> Result<()> {
    let response = client.sessions().list().await?;

    if output.is_text() {
        if response.data.is_empty() {
            println!("No sessions found");
            return Ok(());
        }

        print_table_header(&[("ID", 36), ("TITLE", 25), ("STATUS", 10), ("CREATED", 20)]);

        for session in &response.data {
            let title = session.title.as_deref().unwrap_or("-");
            let status = format!("{:?}", session.status).to_lowercase();
            print_table_row(&[
                (&session.id, 36),
                (title, 25),
                (&status, 10),
                (&session.created_at, 20),
            ]);
        }
    } else {
        // Convert to JSON for non-text output
        let data: Vec<serde_json::Value> = response
            .data
            .iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "organization_id": s.organization_id,
                    "agent_id": s.agent_id,
                    "title": s.title,
                    "tags": s.tags,
                    "model_id": s.model_id,
                    "status": format!("{:?}", s.status).to_lowercase(),
                    "created_at": s.created_at,
                    "updated_at": s.updated_at,
                })
            })
            .collect();
        output.print_value(&serde_json::json!({ "data": data }));
    }

    Ok(())
}

async fn get(client: &Everruns, output: OutputFormat, session_id: String) -> Result<()> {
    let session = client
        .sessions()
        .get(&session_id)
        .await
        .map_err(|e| anyhow::anyhow!("Session not found: {} ({})", session_id, e))?;

    if output.is_text() {
        print_field("ID", &session.id);
        if let Some(agent_id) = &session.agent_id {
            print_field("Agent", agent_id);
        }
        let status = format!("{:?}", session.status).to_lowercase();
        print_field("Status", &status);
        if let Some(title) = &session.title {
            print_field("Title", title);
        }
        if !session.tags.is_empty() {
            print_field("Tags", &session.tags.join(", "));
        }
        print_field("Created", &session.created_at);
    } else {
        output.print_value(&session);
    }

    Ok(())
}

async fn watch(client: &Everruns, output: OutputFormat, session_id: String) -> Result<()> {
    // Verify session exists
    let session = client
        .sessions()
        .get(&session_id)
        .await
        .map_err(|e| anyhow::anyhow!("Session not found: {} ({})", session_id, e))?;

    if output.is_text() {
        let title = session.title.as_deref().unwrap_or("(untitled)");
        eprintln!("Watching session {} [{}]", session_id, title);
        eprintln!("Press Ctrl+C to stop\n");
    }

    let mut stream = client.events().stream(&session_id);

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                if output.is_text() {
                    eprintln!("\nStopped watching");
                }
                return Ok(());
            }
            item = stream.next() => {
                match item {
                    Some(Ok(event)) => {
                        if output.is_text() {
                            format_event_text(&event.event_type, &event.data, &event.ts);
                        } else {
                            let event_json = serde_json::json!({
                                "id": event.id,
                                "type": event.event_type,
                                "ts": event.ts,
                                "session_id": event.session_id,
                                "data": event.data,
                            });
                            output.print_value(&event_json);
                        }
                    }
                    Some(Err(e)) => {
                        eprintln!("Stream error: {e}");
                    }
                    None => {
                        // Stream ended (server closed connection)
                        if output.is_text() {
                            eprintln!("Stream ended");
                        }
                        return Ok(());
                    }
                }
            }
        }
    }
}

/// Format a single event for human-readable text output.
fn format_event_text(event_type: &str, data: &serde_json::Value, ts: &str) {
    match event_type {
        "turn.started" => {
            eprintln!("[{ts}] Turn started");
        }
        "turn.completed" => {
            eprintln!("[{ts}] Turn completed");
        }
        "turn.failed" => {
            let error = data
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            eprintln!("[{ts}] Turn failed: {error}");
        }
        "turn.cancelled" => {
            eprintln!("[{ts}] Turn cancelled");
        }
        "tool.started" => {
            // ToolStartedData has tool_call.name
            let name = data
                .get("tool_call")
                .and_then(|tc| tc.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");
            eprintln!("[{ts}] Tool started: {name}");
        }
        "tool.completed" => {
            // ToolCompletedData has tool_name and status
            let name = data
                .get("tool_name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");
            let status = data
                .get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");
            eprintln!("[{ts}] Tool completed: {name} [{status}]");
        }
        "tool.call_requested" => {
            let name = data
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");
            eprintln!("[{ts}] Tool call requested: {name}");
        }
        "output.message.completed" => {
            let content = data
                .get("content")
                .or_else(|| data.get("message").and_then(|m| m.get("content")));
            if let Some(parts) = content.and_then(|c| c.as_array()) {
                for part in parts {
                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                        println!("{text}");
                    }
                }
            }
        }
        "output.message.delta" => {
            // OutputMessageDeltaData has delta: String (plain text chunk)
            if let Some(text) = data.get("delta").and_then(|d| d.as_str()) {
                print!("{text}");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        }
        "reason.started" | "reason.completed" | "act.started" | "act.completed" => {
            let phase = event_type.replace('.', " ");
            let phase = capitalize_first(&phase);
            eprintln!("[{ts}] {phase}");
        }
        "input.message" => {
            // InputMessageData has message.content: Vec<ContentPart>
            let text = data
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array())
                .and_then(|parts| {
                    parts
                        .iter()
                        .find_map(|p| p.get("text").and_then(|t| t.as_str()))
                });
            if let Some(text) = text {
                let preview = truncate_str(text, 120);
                eprintln!("[{ts}] Input: {preview}");
            } else {
                eprintln!("[{ts}] Input message");
            }
        }
        "session.started" | "session.activated" | "session.idled" => {
            let label = event_type.replace('.', " ");
            let label = capitalize_first(&label);
            eprintln!("[{ts}] {label}");
        }
        "subagent.spawned" => {
            eprintln!("[{ts}] Subagent spawned");
        }
        "subagent.completed" => {
            eprintln!("[{ts}] Subagent completed");
        }
        "subagent.failed" => {
            eprintln!("[{ts}] Subagent failed");
        }
        "subagent.cancelled" => {
            eprintln!("[{ts}] Subagent cancelled");
        }
        _ => {
            eprintln!("[{ts}] {event_type}");
        }
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{truncated}...")
    }
}

// TODO: Replace direct HTTP with everruns-sdk once export is supported.
// Tracking issue: https://github.com/everruns/everruns/issues/1180
async fn export(
    api_url: &str,
    api_key: &str,
    output: OutputFormat,
    quiet: bool,
    session_id: String,
    file_path: Option<String>,
) -> Result<()> {
    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{}/v1/sessions/{}/export", api_url, session_id))
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .context("Failed to request session export")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Export failed ({}): {}", status, body);
    }

    let body = resp.text().await.context("Failed to read export body")?;

    if let Some(path) = file_path {
        std::fs::write(&path, &body).with_context(|| format!("Failed to write to {}", path))?;
        if output.is_text() && !quiet {
            let line_count = body.lines().count();
            eprintln!("Exported {} messages to {}", line_count, path);
        }
    } else {
        print!("{}", body);
    }

    Ok(())
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_secrets_valid() {
        let raw = vec![
            "KEY1=value1".to_string(),
            "KEY2=value2".to_string(),
            "KEY3=val=with=equals".to_string(),
        ];
        let secrets = parse_secrets(&raw).unwrap();
        assert_eq!(secrets.len(), 3);
        assert_eq!(secrets["KEY1"], "value1");
        assert_eq!(secrets["KEY3"], "val=with=equals");
    }

    #[test]
    fn test_parse_secrets_empty() {
        let secrets = parse_secrets(&[]).unwrap();
        assert!(secrets.is_empty());
    }

    #[test]
    fn test_parse_secrets_missing_equals() {
        let raw = vec!["NOEQUALS".to_string()];
        assert!(parse_secrets(&raw).is_err());
    }

    #[test]
    fn test_parse_secrets_empty_key() {
        let raw = vec!["=value".to_string()];
        assert!(parse_secrets(&raw).is_err());
    }

    #[test]
    fn test_parse_secrets_empty_value_allowed() {
        let raw = vec!["KEY=".to_string()];
        let secrets = parse_secrets(&raw).unwrap();
        assert_eq!(secrets["KEY"], "");
    }

    #[test]
    fn test_parse_secrets_duplicate_key() {
        let raw = vec!["KEY=value1".to_string(), "KEY=value2".to_string()];
        let result = parse_secrets(&raw);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Duplicate secret key")
        );
    }

    #[test]
    fn test_parse_budget_limits_usd_default() {
        let specs = parse_budget_limits(&["10".into()], &[]).unwrap();
        assert_eq!(specs.len(), 1);
        assert!((specs[0].limit - 10.0).abs() < f64::EPSILON);
        assert_eq!(specs[0].currency, "usd");
        assert!(specs[0].soft_limit.is_none());
    }

    #[test]
    fn test_parse_budget_limits_explicit_currency() {
        let specs = parse_budget_limits(&["tokens:2000000".into()], &[]).unwrap();
        assert_eq!(specs[0].currency, "tokens");
        assert!((specs[0].limit - 2_000_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_budget_limits_with_soft_limit() {
        let specs = parse_budget_limits(&["usd:10".into()], &["usd:8".into()]).unwrap();
        assert!((specs[0].limit - 10.0).abs() < f64::EPSILON);
        assert_eq!(specs[0].currency, "usd");
        assert!((specs[0].soft_limit.unwrap() - 8.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_budget_limits_soft_default_currency() {
        let specs = parse_budget_limits(&["10".into()], &["8".into()]).unwrap();
        assert_eq!(specs[0].currency, "usd");
        assert!((specs[0].soft_limit.unwrap() - 8.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_budget_limits_multiple() {
        let specs = parse_budget_limits(&["usd:10".into(), "tokens:5000000".into()], &[]).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].currency, "usd");
        assert_eq!(specs[1].currency, "tokens");
    }

    #[test]
    fn test_parse_budget_limits_empty() {
        let specs = parse_budget_limits(&[], &[]).unwrap();
        assert!(specs.is_empty());
    }

    #[test]
    fn test_parse_budget_limits_zero_fails() {
        assert!(parse_budget_limits(&["0".into()], &[]).is_err());
    }

    #[test]
    fn test_parse_budget_limits_soft_exceeds_limit_fails() {
        assert!(parse_budget_limits(&["usd:10".into()], &["usd:15".into()]).is_err());
    }

    #[test]
    fn test_parse_budget_limits_soft_no_matching_limit_fails() {
        assert!(parse_budget_limits(&["usd:10".into()], &["tokens:5000".into()]).is_err());
    }

    #[test]
    fn test_parse_budget_limits_non_numeric_fails() {
        assert!(parse_budget_limits(&["abc".into()], &[]).is_err());
    }

    #[test]
    fn test_format_budget_amount() {
        assert_eq!(format_budget_amount(10.0, "usd"), "$10.00");
        assert_eq!(format_budget_amount(2000000.0, "tokens"), "2000000 tokens");
        assert_eq!(format_budget_amount(50.0, "credits"), "50 credits");
        assert_eq!(format_budget_amount(100.0, "gems"), "100 gems");
    }
}
