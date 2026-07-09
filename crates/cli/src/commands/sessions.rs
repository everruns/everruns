// Session management commands

use crate::output::{OutputFormat, print_field, print_table_header, print_table_row};
use anyhow::{Context, Result};
use clap::{Subcommand, ValueEnum};
use everruns_sdk::{CreateBudgetRequest, Everruns};
use futures::StreamExt;
use std::collections::HashMap;

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum SessionsCommand {
    /// Create a new session
    Create {
        /// Harness ID or name (e.g. harness_xxx or "generic"). Omit to use org default.
        #[arg(long, short = 'H')]
        harness: Option<String>,

        /// Agent ID (optional, e.g. agent_xxx)
        #[arg(long, short)]
        agent: Option<String>,

        /// Session title
        #[arg(long)]
        title: Option<String>,

        /// Session locale (BCP 47, e.g. uk-UA)
        #[arg(long)]
        locale: Option<String>,

        /// Model ID override (e.g. mod_xxx)
        #[arg(long)]
        model: Option<String>,

        /// Resident agent identity ID for unattended/background execution
        #[arg(long = "agent-identity")]
        agent_identity: Option<String>,

        /// Session-level system prompt override
        #[arg(long = "system-prompt")]
        system_prompt: Option<String>,

        /// Session tag (repeatable)
        #[arg(long = "tag", short = 't')]
        tags: Vec<String>,

        /// Session capability (repeatable). Format: REF or REF=JSON_CONFIG.
        #[arg(long = "capability", value_name = "REF[=JSON]")]
        capabilities: Vec<String>,

        /// Session client hint (repeatable). Format: KEY=JSON_VALUE.
        #[arg(long = "hint", value_name = "KEY=JSON")]
        hints: Vec<String>,

        /// Session client hints JSON object
        #[arg(long = "hints-json", value_name = "JSON")]
        hints_json: Option<String>,

        /// Network allow pattern (repeatable)
        #[arg(long = "network-allow", value_name = "PATTERN")]
        network_allow: Vec<String>,

        /// Network block pattern (repeatable)
        #[arg(long = "network-block", value_name = "PATTERN")]
        network_block: Vec<String>,

        /// Maximum LLM iterations per turn
        #[arg(long = "max-iterations")]
        max_iterations: Option<usize>,

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

    /// Export session messages as JSONL or an ATIF trajectory
    Export {
        /// Session ID (e.g. session_xxx)
        session: String,

        /// Output file path (defaults to stdout)
        #[arg(long, short)]
        output: Option<String>,

        /// Export format: `jsonl` (one message per line, default) or `atif`
        /// (a single ATIF trajectory JSON document)
        #[arg(long, value_enum, default_value_t = ExportFormat::Jsonl)]
        format: ExportFormat,
    },
}

/// Session export wire format selected by `sessions export --format`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum ExportFormat {
    /// Newline-delimited JSON, one message per line.
    Jsonl,
    /// A single ATIF (Agent Trajectory Interchange Format) JSON document.
    Atif,
}

impl ExportFormat {
    /// The `format` query-parameter value understood by the export API.
    fn as_query_value(self) -> &'static str {
        match self {
            ExportFormat::Jsonl => "jsonl",
            ExportFormat::Atif => "atif",
        }
    }
}

pub async fn run(
    command: SessionsCommand,
    client: &Everruns,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    match command {
        SessionsCommand::Create {
            harness,
            agent,
            title,
            locale,
            model,
            agent_identity,
            system_prompt,
            tags,
            capabilities,
            hints,
            hints_json,
            network_allow,
            network_block,
            max_iterations,
            secrets,
            budget_limits,
            budget_soft_limits,
        } => {
            create(
                client,
                api_url,
                api_key,
                org_id,
                output,
                quiet,
                harness,
                agent,
                title,
                locale,
                model,
                agent_identity,
                system_prompt,
                tags,
                capabilities,
                hints,
                hints_json,
                network_allow,
                network_block,
                max_iterations,
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
            format,
        } => {
            export(
                client, api_url, api_key, org_id, output, quiet, session, file_path, format,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn create(
    client: &Everruns,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    output: OutputFormat,
    quiet: bool,
    harness: Option<String>,
    agent_id: Option<String>,
    title: Option<String>,
    locale: Option<String>,
    model_id: Option<String>,
    agent_identity_id: Option<String>,
    system_prompt: Option<String>,
    tags: Vec<String>,
    raw_capabilities: Vec<String>,
    raw_hints: Vec<String>,
    raw_hints_json: Option<String>,
    network_allow: Vec<String>,
    network_block: Vec<String>,
    max_iterations: Option<usize>,
    raw_secrets: Vec<String>,
    raw_budget_limits: Vec<String>,
    raw_budget_soft_limits: Vec<String>,
) -> Result<()> {
    let secrets = parse_secrets(&raw_secrets)?;
    let budget_specs = parse_budget_limits(&raw_budget_limits, &raw_budget_soft_limits)?;
    let body = build_create_session_body(CreateSessionArgs {
        harness,
        agent_id,
        title,
        locale,
        model_id,
        agent_identity_id,
        system_prompt,
        tags,
        raw_capabilities,
        raw_hints,
        raw_hints_json,
        network_allow,
        network_block,
        max_iterations,
    })?;

    let session = create_session_raw(api_url, api_key, org_id, &body).await?;
    let session_id = session
        .get("id")
        .and_then(|id| id.as_str())
        .context("Create session response did not include id")?
        .to_string();

    // Store secrets after session creation
    if !secrets.is_empty() {
        client
            .sessions()
            .set_secrets(&session_id, &secrets)
            .await
            .with_context(|| {
                format!(
                    "Failed to store {} secrets for session {}",
                    secrets.len(),
                    session_id
                )
            })?;
    }

    // Create budgets after session creation
    let mut created_budgets = Vec::new();
    for spec in &budget_specs {
        let mut req = CreateBudgetRequest::new("session", &session_id, &spec.currency, spec.limit);
        if let Some(soft) = spec.soft_limit {
            req = req.soft_limit(soft);
        }
        let budget = client.budgets().create(req).await.with_context(|| {
            format!("Session {} created but budget creation failed", session_id)
        })?;
        created_budgets.push(serde_json::to_value(&budget)?);
    }

    if output.is_text() {
        if quiet {
            println!("{}", session_id);
        } else {
            println!("Created session: {}", session_id);
            if let Some(agent) = session.get("agent_id").and_then(|v| v.as_str()) {
                print_field("Agent", agent);
            }
            if let Some(identity) = session.get("agent_identity_id").and_then(|v| v.as_str()) {
                print_field("Identity", identity);
            }
            if let Some(status) = session.get("status").and_then(value_as_status) {
                print_field("Status", &status);
            }
            if let Some(features) = session.get("features").and_then(|v| v.as_array())
                && !features.is_empty()
            {
                let features = features
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                if !features.is_empty() {
                    print_field("Features", &features);
                }
            }
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
        let mut json = session;
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

struct CreateSessionArgs {
    harness: Option<String>,
    agent_id: Option<String>,
    title: Option<String>,
    locale: Option<String>,
    model_id: Option<String>,
    agent_identity_id: Option<String>,
    system_prompt: Option<String>,
    tags: Vec<String>,
    raw_capabilities: Vec<String>,
    raw_hints: Vec<String>,
    raw_hints_json: Option<String>,
    network_allow: Vec<String>,
    network_block: Vec<String>,
    max_iterations: Option<usize>,
}

fn build_create_session_body(args: CreateSessionArgs) -> Result<serde_json::Value> {
    let mut body = serde_json::Map::new();

    if let Some(h) = args.harness {
        if is_prefixed_id(&h, "harness") {
            body.insert("harness_id".to_string(), serde_json::json!(h));
        } else {
            body.insert("harness_name".to_string(), serde_json::json!(h));
        }
    }
    insert_opt_string(&mut body, "agent_id", args.agent_id);
    insert_opt_string(&mut body, "agent_identity_id", args.agent_identity_id);
    insert_opt_string(&mut body, "title", args.title);
    insert_opt_string(&mut body, "locale", args.locale);
    insert_opt_string(&mut body, "model_id", args.model_id);
    insert_opt_string(&mut body, "system_prompt", args.system_prompt);

    if !args.tags.is_empty() {
        body.insert("tags".to_string(), serde_json::json!(args.tags));
    }
    if !args.raw_capabilities.is_empty() {
        body.insert(
            "capabilities".to_string(),
            serde_json::Value::Array(parse_capabilities(&args.raw_capabilities)?),
        );
    }

    let hints = parse_hints(args.raw_hints_json.as_deref(), &args.raw_hints)?;
    if !hints.is_empty() {
        body.insert("hints".to_string(), serde_json::Value::Object(hints));
    }

    if !args.network_allow.is_empty() || !args.network_block.is_empty() {
        let mut network_access = serde_json::Map::new();
        if !args.network_allow.is_empty() {
            network_access.insert("allowed".to_string(), serde_json::json!(args.network_allow));
        }
        if !args.network_block.is_empty() {
            network_access.insert("blocked".to_string(), serde_json::json!(args.network_block));
        }
        body.insert(
            "network_access".to_string(),
            serde_json::Value::Object(network_access),
        );
    }

    if let Some(max_iterations) = args.max_iterations {
        if max_iterations == 0 {
            anyhow::bail!("--max-iterations must be greater than zero");
        }
        body.insert(
            "max_iterations".to_string(),
            serde_json::json!(max_iterations),
        );
    }

    Ok(serde_json::Value::Object(body))
}

fn is_prefixed_id(value: &str, prefix: &str) -> bool {
    let expected = format!("{prefix}_");
    let Some(rest) = value.strip_prefix(&expected) else {
        return false;
    };
    rest.len() == 32 && rest.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

fn insert_opt_string(
    body: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: Option<String>,
) {
    if let Some(value) = value {
        body.insert(key.to_string(), serde_json::json!(value));
    }
}

fn parse_capabilities(raw: &[String]) -> Result<Vec<serde_json::Value>> {
    raw.iter()
        .map(|entry| {
            let (capability_ref, config) = if let Some((left, right)) = entry.split_once('=') {
                if left.is_empty() {
                    anyhow::bail!("Capability ref cannot be empty: {entry}");
                }
                let config = serde_json::from_str(right)
                    .with_context(|| format!("Invalid capability JSON config: {entry}"))?;
                (left, config)
            } else {
                if entry.is_empty() {
                    anyhow::bail!("Capability ref cannot be empty");
                }
                (entry.as_str(), serde_json::json!({}))
            };
            Ok(serde_json::json!({
                "ref": capability_ref,
                "config": config,
            }))
        })
        .collect()
}

fn parse_hints(
    hints_json: Option<&str>,
    raw_hints: &[String],
) -> Result<serde_json::Map<String, serde_json::Value>> {
    let mut hints = if let Some(raw) = hints_json {
        let value: serde_json::Value =
            serde_json::from_str(raw).context("Invalid --hints-json value")?;
        value
            .as_object()
            .cloned()
            .context("--hints-json must be a JSON object")?
    } else {
        serde_json::Map::new()
    };

    for entry in raw_hints {
        let (key, value) = entry
            .split_once('=')
            .with_context(|| format!("Invalid hint format (expected KEY=JSON): {entry}"))?;
        if key.is_empty() {
            anyhow::bail!("Hint key cannot be empty: {entry}");
        }
        if hints.contains_key(key) {
            anyhow::bail!("Duplicate hint key: {key}");
        }
        let value = serde_json::from_str(value)
            .with_context(|| format!("Invalid JSON value for hint '{key}'"))?;
        hints.insert(key.to_string(), value);
    }

    Ok(hints)
}

async fn create_session_raw(
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    body: &serde_json::Value,
) -> Result<serde_json::Value> {
    let http = reqwest::Client::new();
    let mut req = http
        .post(format!("{}/v1/sessions", api_url.trim_end_matches('/')))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(body);
    let env_org = std::env::var("EVERRUNS_ORG_ID").ok();
    if let Some(org) = org_id.or(env_org.as_deref()) {
        req = req.header("X-Org-Id", org);
    }

    let resp = req.send().await.context("Failed to create session")?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Create session failed ({}): {}", status, body);
    }

    resp.json()
        .await
        .context("Failed to parse create session response")
}

fn value_as_status(value: &serde_json::Value) -> Option<String> {
    value.as_str().map(ToOwned::to_owned).or_else(|| {
        value
            .get("status")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
    })
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
    // Parse hard limits, rejecting duplicate currencies
    let mut seen_currencies = std::collections::HashSet::new();
    let mut specs: Vec<BudgetSpec> = Vec::new();
    for entry in limits {
        let (currency, limit) = parse_currency_amount(entry)?;
        if limit <= 0.0 {
            anyhow::bail!("Budget limit must be positive: {entry}");
        }
        if !seen_currencies.insert(currency.clone()) {
            anyhow::bail!("Duplicate --budget-limit for currency '{currency}'");
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
                if soft > spec.limit {
                    anyhow::bail!(
                        "Soft limit ({soft}) must be at most limit ({}) for currency '{currency}'",
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
    // Tracks whether this stream has produced task.* lifecycle events, so the
    // legacy subagent.* fallback rendering can switch off (see format_event_text).
    let mut saw_task_events = false;

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
                            format_event_text(&event.event_type, &event.data, &event.ts, &mut saw_task_events);
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
///
/// `saw_task_events` is stream-scoped state: once a `task.*` lifecycle event
/// has been seen, legacy `subagent.*` events are skipped (they duplicate the
/// task lifecycle on servers that emit both). Until then they render with the
/// legacy lines so older servers that only emit `subagent.*` stay readable.
fn format_event_text(
    event_type: &str,
    data: &serde_json::Value,
    ts: &str,
    saw_task_events: &mut bool,
) {
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
        // Session task lifecycle: events carry a full task snapshot
        // (specs/session-tasks.md). Render kind, name, state, and detail.
        "task.created" | "task.updated" => {
            *saw_task_events = true;
            eprintln!("{}", format_task_event_line(event_type, data, ts));
        }
        "task.message.sent" | "task.message.received" => {
            *saw_task_events = true;
            eprintln!("{}", format_task_message_line(event_type, data, ts));
        }
        // Legacy subagent.* events duplicate the task.* lifecycle above
        // (subagents are session tasks now). On servers that emit both, skip
        // them to avoid double-printing; on older servers that only emit
        // subagent.* (no task.* seen yet), keep the legacy lines.
        "subagent.spawned" | "subagent.completed" | "subagent.failed" | "subagent.cancelled" => {
            if !*saw_task_events {
                let label = capitalize_first(&event_type.replace('.', " "));
                eprintln!("[{ts}] {label}");
            }
        }
        _ => {
            eprintln!("[{ts}] {event_type}");
        }
    }
}

/// Render a `task.created`/`task.updated` line from the event's task snapshot.
fn format_task_event_line(event_type: &str, data: &serde_json::Value, ts: &str) -> String {
    let task = data.get("task");
    let get = |key: &str| {
        task.and_then(|t| t.get(key))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
    };
    let state = get("state");
    let name = get("display_name");
    let kind = get("kind");
    let detail = task
        .and_then(|t| t.get("state_detail"))
        .and_then(|v| v.as_str())
        .map(|d| format!(" — {d}"))
        .unwrap_or_default();
    let verb = if event_type == "task.created" {
        "Task created"
    } else {
        "Task"
    };
    format!("[{ts}] {verb} [{kind}] {name}: {state}{detail}")
}

/// Render a `task.message.sent`/`task.message.received` line with a
/// direction tag and a truncated text preview.
fn format_task_message_line(event_type: &str, data: &serde_json::Value, ts: &str) -> String {
    let direction = if event_type == "task.message.sent" {
        "→ task"
    } else {
        "task →"
    };
    let text = data
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
        .and_then(|parts| {
            parts
                .iter()
                .find_map(|p| p.get("text").and_then(|t| t.as_str()))
        });
    match text {
        Some(text) => {
            let preview = truncate_str(text, 120);
            format!("[{ts}] Message {direction}: {preview}")
        }
        None => format!("[{ts}] Message {direction}"),
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

#[allow(clippy::too_many_arguments)]
async fn export(
    client: &Everruns,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    output: OutputFormat,
    quiet: bool,
    session_id: String,
    file_path: Option<String>,
    format: ExportFormat,
) -> Result<()> {
    // JSONL stays on the SDK path (unchanged default). ATIF isn't exposed by
    // the SDK's `export()` yet, so hit the export endpoint directly with the
    // `format` query parameter. See EVE-685.
    let body = match format {
        ExportFormat::Jsonl => client
            .sessions()
            .export(&session_id)
            .await
            .context("Failed to export session")?,
        ExportFormat::Atif => export_raw(api_url, api_key, org_id, &session_id, format).await?,
    };

    if let Some(path) = file_path {
        std::fs::write(&path, &body).with_context(|| format!("Failed to write to {}", path))?;
        if output.is_text() && !quiet {
            // JSONL is line-per-message; ATIF is a single JSON document, so a
            // line count would be misleading. Report bytes for ATIF instead.
            match format {
                ExportFormat::Jsonl => {
                    eprintln!("Exported {} messages to {}", body.lines().count(), path)
                }
                ExportFormat::Atif => {
                    eprintln!(
                        "Exported ATIF trajectory ({} bytes) to {}",
                        body.len(),
                        path
                    )
                }
            }
        }
    } else {
        print!("{}", body);
    }

    Ok(())
}

/// Fetch a session export directly from `GET /v1/sessions/{id}/export` with an
/// explicit `format`, for formats the SDK client does not yet expose.
async fn export_raw(
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    session_id: &str,
    format: ExportFormat,
) -> Result<String> {
    let http = reqwest::Client::new();
    // `format` is a fixed enum-derived token (`jsonl`/`atif`), so it is safe to
    // interpolate directly into the query string.
    let mut req = http
        .get(format!(
            "{}/v1/sessions/{}/export?format={}",
            api_url.trim_end_matches('/'),
            session_id,
            format.as_query_value(),
        ))
        .header("Authorization", format!("Bearer {}", api_key));
    let env_org = std::env::var("EVERRUNS_ORG_ID").ok();
    if let Some(org) = org_id.or(env_org.as_deref()) {
        req = req.header("X-Org-Id", org);
    }

    let resp = req.send().await.context("Failed to export session")?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Export session failed ({}): {}", status, body);
    }

    resp.text()
        .await
        .context("Failed to read session export response")
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
    fn task_created_line_uses_created_verb_and_snapshot_fields() {
        let data = serde_json::json!({
            "task": {
                "kind": "subagent",
                "display_name": "Test Runner",
                "state": "queued",
                "state_detail": null
            }
        });
        assert_eq!(
            format_task_event_line("task.created", &data, "t0"),
            "[t0] Task created [subagent] Test Runner: queued"
        );
    }

    #[test]
    fn task_updated_line_includes_state_detail() {
        let data = serde_json::json!({
            "task": {
                "kind": "monitor",
                "display_name": "Build Watch",
                "state": "running",
                "state_detail": "iteration 4/10"
            }
        });
        assert_eq!(
            format_task_event_line("task.updated", &data, "t1"),
            "[t1] Task [monitor] Build Watch: running — iteration 4/10"
        );
    }

    #[test]
    fn task_event_line_tolerates_missing_snapshot() {
        assert_eq!(
            format_task_event_line("task.updated", &serde_json::json!({}), "t2"),
            "[t2] Task [unknown] unknown: unknown"
        );
    }

    #[test]
    fn task_message_lines_render_direction_and_preview() {
        let data = serde_json::json!({
            "task_id": "task_x",
            "message": { "content": [ { "type": "text", "text": "hello there" } ] }
        });
        assert_eq!(
            format_task_message_line("task.message.sent", &data, "t3"),
            "[t3] Message → task: hello there"
        );
        assert_eq!(
            format_task_message_line("task.message.received", &data, "t3"),
            "[t3] Message task →: hello there"
        );
    }

    #[test]
    fn task_message_line_truncates_long_text() {
        let long = "x".repeat(200);
        let data = serde_json::json!({
            "message": { "content": [ { "type": "text", "text": long } ] }
        });
        let line = format_task_message_line("task.message.sent", &data, "t4");
        assert!(line.ends_with("..."));
        assert!(line.len() < 200);
    }

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
    fn test_parse_budget_limits_soft_equals_limit_ok() {
        let specs = parse_budget_limits(&["usd:10".into()], &["usd:10".into()]).unwrap();
        assert!((specs[0].soft_limit.unwrap() - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_budget_limits_duplicate_currency_fails() {
        assert!(parse_budget_limits(&["usd:10".into(), "usd:5".into()], &[]).is_err());
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
    fn test_parse_capabilities_plain_and_configured() {
        let parsed =
            parse_capabilities(&["web_fetch".into(), "filesystem={\"readonly\":true}".into()])
                .unwrap();
        assert_eq!(parsed[0]["ref"], "web_fetch");
        assert_eq!(parsed[0]["config"], serde_json::json!({}));
        assert_eq!(parsed[1]["ref"], "filesystem");
        assert_eq!(parsed[1]["config"], serde_json::json!({"readonly": true}));
    }

    #[test]
    fn test_parse_capabilities_rejects_invalid_json() {
        assert!(parse_capabilities(&["web_fetch={bad}".into()]).is_err());
    }

    #[test]
    fn test_parse_hints_merges_json_and_flags() {
        let parsed = parse_hints(
            Some("{\"rich_media\":true}"),
            &["setup_connection=true".into(), "max_items=3".into()],
        )
        .unwrap();
        assert_eq!(parsed["rich_media"], serde_json::json!(true));
        assert_eq!(parsed["setup_connection"], serde_json::json!(true));
        assert_eq!(parsed["max_items"], serde_json::json!(3));
    }

    #[test]
    fn test_parse_hints_rejects_duplicate() {
        assert!(
            parse_hints(
                Some("{\"setup_connection\":false}"),
                &["setup_connection=true".into()]
            )
            .is_err()
        );
    }

    #[test]
    fn test_build_create_session_body_new_fields() {
        let body = build_create_session_body(CreateSessionArgs {
            harness: Some("generic".into()),
            agent_id: Some("agent_abc".into()),
            title: Some("Debug".into()),
            locale: Some("uk-UA".into()),
            model_id: Some("model_abc".into()),
            agent_identity_id: Some("identity_abc".into()),
            system_prompt: Some("Be concise".into()),
            tags: vec!["bug".into()],
            raw_capabilities: vec!["web_fetch={\"timeout\":10}".into()],
            raw_hints: vec!["setup_connection=true".into()],
            raw_hints_json: Some("{\"rich_media\":true}".into()),
            network_allow: vec!["api.example.com".into()],
            network_block: vec!["internal.example.com".into()],
            max_iterations: Some(8),
        })
        .unwrap();

        assert_eq!(body["harness_name"], "generic");
        assert_eq!(body["agent_id"], "agent_abc");
        assert_eq!(body["agent_identity_id"], "identity_abc");
        assert_eq!(body["locale"], "uk-UA");
        assert_eq!(body["system_prompt"], "Be concise");
        assert_eq!(body["tags"], serde_json::json!(["bug"]));
        assert_eq!(body["capabilities"][0]["ref"], "web_fetch");
        assert_eq!(body["hints"]["setup_connection"], serde_json::json!(true));
        assert_eq!(body["hints"]["rich_media"], serde_json::json!(true));
        assert_eq!(
            body["network_access"]["allowed"],
            serde_json::json!(["api.example.com"])
        );
        assert_eq!(
            body["network_access"]["blocked"],
            serde_json::json!(["internal.example.com"])
        );
        assert_eq!(body["max_iterations"], 8);
    }

    #[test]
    fn test_build_create_session_body_detects_harness_id_strictly() {
        let body = build_create_session_body(CreateSessionArgs {
            harness: Some("harness_00000000000000000000000000000001".into()),
            agent_id: None,
            title: None,
            locale: None,
            model_id: None,
            agent_identity_id: None,
            system_prompt: None,
            tags: vec![],
            raw_capabilities: vec![],
            raw_hints: vec![],
            raw_hints_json: None,
            network_allow: vec![],
            network_block: vec![],
            max_iterations: None,
        })
        .unwrap();

        assert_eq!(
            body["harness_id"],
            "harness_00000000000000000000000000000001"
        );
        assert!(body.get("harness_name").is_none());
    }

    #[test]
    fn test_build_create_session_body_keeps_harness_prefix_names() {
        let body = build_create_session_body(CreateSessionArgs {
            harness: Some("harness_generic".into()),
            agent_id: None,
            title: None,
            locale: None,
            model_id: None,
            agent_identity_id: None,
            system_prompt: None,
            tags: vec![],
            raw_capabilities: vec![],
            raw_hints: vec![],
            raw_hints_json: None,
            network_allow: vec![],
            network_block: vec![],
            max_iterations: None,
        })
        .unwrap();

        assert_eq!(body["harness_name"], "harness_generic");
        assert!(body.get("harness_id").is_none());
    }

    #[test]
    fn test_format_budget_amount() {
        assert_eq!(format_budget_amount(10.0, "usd"), "$10.00");
        assert_eq!(format_budget_amount(2000000.0, "tokens"), "2000000 tokens");
        assert_eq!(format_budget_amount(50.0, "credits"), "50 credits");
        assert_eq!(format_budget_amount(100.0, "gems"), "100 gems");
    }

    #[test]
    fn export_format_maps_to_api_query_value() {
        assert_eq!(ExportFormat::Jsonl.as_query_value(), "jsonl");
        assert_eq!(ExportFormat::Atif.as_query_value(), "atif");
    }

    #[test]
    fn export_command_parses_format_flag_and_defaults_to_jsonl() {
        use clap::Parser;

        #[derive(Parser)]
        struct Wrapper {
            #[command(subcommand)]
            cmd: SessionsCommand,
        }

        // Default (no --format) is JSONL, preserving prior behavior.
        let parsed = Wrapper::parse_from(["x", "export", "session_123"]);
        match parsed.cmd {
            SessionsCommand::Export { format, .. } => assert_eq!(format, ExportFormat::Jsonl),
            _ => panic!("expected export command"),
        }

        // Explicit --format atif is accepted.
        let parsed = Wrapper::parse_from(["x", "export", "session_123", "--format", "atif"]);
        match parsed.cmd {
            SessionsCommand::Export { format, .. } => assert_eq!(format, ExportFormat::Atif),
            _ => panic!("expected export command"),
        }
    }
}
