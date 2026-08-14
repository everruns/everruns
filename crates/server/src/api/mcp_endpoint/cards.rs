// MCP entity cards — UI resources rendered in MCP-Apps-aware hosts.
//
// See `knowledge/ui/mcp-cards.md` for the standard. Highlights:
// - URI scheme: `ui://everruns/{entity}/{public_id}/card`
// - MIME: `text/html`, single self-contained sandboxed document
// - Every entity-controlled string flows through `escape_html`
// - 64 KiB rendered-document cap; oversized cards are rejected
// - Phase 2 mutation actions go through host postMessage → tools/call,
//   so the card itself is never the trust boundary

use chrono::{DateTime, Utc};
use everruns_platform::Agent;
use serde_json::{Value, json};
use std::fmt::Write as _;

// We use writeln! for HTML interpolation lines: it appends '\n' so we
// don't end format strings in `\n` (clippy::write_with_newline).

/// Hard cap on rendered HTML size — rejects rather than truncates.
pub const MAX_CARD_BYTES: usize = 64 * 1024;

/// Entity kinds that can have cards. Singular kebab-case form is used in
/// the URI scheme.
#[derive(Debug, Clone, Copy)]
pub enum EntityKind {
    Agent,
}

impl EntityKind {
    fn slug(self) -> &'static str {
        match self {
            EntityKind::Agent => "agent",
        }
    }
}

/// One stat row in the card body.
#[derive(Debug, Clone)]
pub struct StatItem {
    pub label: String,
    pub value: String,
}

/// In-memory model of a card. Adapter modules (`AgentCard`, future
/// `SessionCard`, etc.) build one of these and hand it to `render_html`.
#[derive(Debug, Clone)]
pub struct EntityCard {
    pub kind: EntityKind,
    /// External prefixed id (e.g. `agent_…`). Used to derive the `ui://`
    /// URI and shown verbatim on the card.
    pub public_id: String,
    /// Header line, typically the display name with name fallback.
    pub title: String,
    /// Optional second header line, e.g. machine-name when display name is
    /// set.
    pub subtitle: Option<String>,
    /// Optional human description (raw text, not Markdown — host-side
    /// link rendering is deferred).
    pub description: Option<String>,
    /// Status badge text (e.g. `active`, `archived`).
    pub status: String,
    /// Tags rendered as small chips.
    pub tags: Vec<String>,
    /// Stat rows. Order is preserved.
    pub stats: Vec<StatItem>,
    /// Footer lines (e.g. `Created Jan 02, 2026`).
    pub footer_lines: Vec<String>,
}

/// Build the `ui://everruns/{entity}/{public_id}/card` URI for a card.
pub fn card_uri(kind: EntityKind, public_id: &str) -> String {
    format!("ui://everruns/{}/{}/card", kind.slug(), public_id)
}

/// HTML-escape a string. Used for every entity-controlled interpolation.
pub fn escape_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Render the card to a self-contained HTML document. Returns `None` if
/// the rendered size exceeds `MAX_CARD_BYTES`. Callers (currently
/// `card_tool_content` → `tool_agent_get_card`) surface this as a tool
/// error (`isError: true`) rather than truncating the document or
/// silently degrading to a text-only result; see `knowledge/ui/mcp-cards.md`.
pub fn render_html(card: &EntityCard) -> Option<String> {
    let mut html = String::with_capacity(2048);

    // Inline CSP plus the required document chrome. The CSP blocks every
    // network fetch (`default-src 'none'` + `connect-src 'none'`) and
    // permits only inline style and (future phase-2) inline script for
    // action wiring. Several CSP directives — notably `frame-ancestors`
    // — are not enforceable when the policy is delivered via
    // `<meta http-equiv>`, so clickjacking and origin isolation are
    // handled by the host's iframe `sandbox` attribute (see
    // `knowledge/ui/mcp-cards.md`), not by CSP.
    html.push_str("<!doctype html>\n");
    html.push_str("<html lang=\"en\"><head><meta charset=\"utf-8\">\n");
    html.push_str(
        "<meta http-equiv=\"Content-Security-Policy\" \
         content=\"default-src 'none'; style-src 'unsafe-inline'; \
         script-src 'unsafe-inline'; img-src data:; \
         connect-src 'none'\">",
    );
    html.push_str("<meta name=\"viewport\" content=\"width=360\">\n");
    let _ = writeln!(
        html,
        "<title>{}</title>",
        escape_html(&format!("Everruns: {}", card.title))
    );

    html.push_str(STYLE_BLOCK);
    html.push_str("</head><body><article class=\"card\">\n");

    // Header
    html.push_str("<header class=\"card-header\">\n");
    let _ = writeln!(
        html,
        "<div class=\"card-kind\">{}</div>",
        escape_html(kind_label(card.kind))
    );
    let _ = writeln!(
        html,
        "<h1 class=\"card-title\">{}</h1>",
        escape_html(&card.title)
    );
    if let Some(sub) = &card.subtitle {
        let _ = writeln!(
            html,
            "<div class=\"card-subtitle\">{}</div>",
            escape_html(sub)
        );
    }
    let _ = writeln!(
        html,
        "<div class=\"card-status status-{}\">{}</div>",
        escape_html(&status_class(&card.status)),
        escape_html(&card.status)
    );
    let _ = writeln!(
        html,
        "<code class=\"card-id\">{}</code>",
        escape_html(&card.public_id)
    );
    html.push_str("</header>\n");

    // Description
    if let Some(desc) = &card.description {
        let _ = writeln!(
            html,
            "<p class=\"card-description\">{}</p>",
            escape_html(desc)
        );
    }

    // Tags
    if !card.tags.is_empty() {
        html.push_str("<div class=\"card-tags\">\n");
        for tag in &card.tags {
            let _ = write!(html, "<span class=\"tag\">{}</span>", escape_html(tag));
        }
        html.push_str("</div>\n");
    }

    // Stats
    if !card.stats.is_empty() {
        html.push_str("<dl class=\"card-stats\">\n");
        for stat in &card.stats {
            let _ = writeln!(
                html,
                "<div class=\"stat\"><dt>{}</dt><dd>{}</dd></div>",
                escape_html(&stat.label),
                escape_html(&stat.value)
            );
        }
        html.push_str("</dl>\n");
    }

    // Footer
    if !card.footer_lines.is_empty() {
        html.push_str("<footer class=\"card-footer\">\n");
        for line in &card.footer_lines {
            let _ = write!(html, "<div>{}</div>", escape_html(line));
        }
        html.push_str("</footer>\n");
    }

    // Reserved slot for future actions. Empty in v1; present so adapters
    // and host CSS can target a stable selector.
    html.push_str("<div class=\"card-actions\" data-card-actions></div>\n");

    html.push_str("</article></body></html>\n");

    if html.len() > MAX_CARD_BYTES {
        return None;
    }
    Some(html)
}

/// Build the `tools/call` content array for a card (resource + summary).
pub fn card_tool_content(card: &EntityCard, summary: &str) -> Result<Value, String> {
    let html = render_html(card).ok_or_else(|| {
        format!(
            "Card HTML exceeded {} byte cap for {}",
            MAX_CARD_BYTES, card.public_id
        )
    })?;
    let uri = card_uri(card.kind, &card.public_id);
    Ok(json!([
        {
            "type": "resource",
            "resource": {
                "uri": uri,
                "mimeType": "text/html",
                "text": html,
            }
        },
        { "type": "text", "text": summary }
    ]))
}

fn kind_label(kind: EntityKind) -> &'static str {
    match kind {
        EntityKind::Agent => "Agent",
    }
}

fn status_class(status: &str) -> String {
    status
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Inline style block. Kept centralised so all card kinds look identical.
const STYLE_BLOCK: &str = r#"<style>
:root { color-scheme: light dark; }
* { box-sizing: border-box; }
body {
  margin: 0;
  font: 13px/1.45 -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
  background: transparent;
  color: #111;
}
@media (prefers-color-scheme: dark) { body { color: #eee; } }
.card {
  width: 360px;
  max-width: 100%;
  padding: 16px;
  border-radius: 12px;
  border: 1px solid rgba(127,127,127,0.25);
  background: rgba(255,255,255,0.6);
}
@media (prefers-color-scheme: dark) {
  .card { background: rgba(20,20,22,0.6); }
}
.card-header { display: grid; grid-template-columns: auto 1fr; gap: 4px 10px; align-items: baseline; }
.card-kind { grid-column: 1 / -1; font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; opacity: 0.6; }
.card-title { grid-column: 1 / -1; font-size: 18px; font-weight: 600; margin: 0; }
.card-subtitle { grid-column: 1 / -1; font-size: 12px; opacity: 0.7; }
.card-id { grid-column: 1 / -1; font-size: 10px; opacity: 0.55; word-break: break-all; }
.card-status {
  grid-column: 1 / -1; justify-self: start;
  font-size: 11px; padding: 2px 8px; border-radius: 999px;
  border: 1px solid currentColor; opacity: 0.85;
}
.status-active { color: #1a7f37; }
.status-archived { color: #9a6700; }
.status-deleted { color: #cf222e; }
.card-description { margin: 12px 0 0; font-size: 13px; opacity: 0.9; white-space: pre-wrap; }
.card-tags { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 12px; }
.tag { font-size: 11px; padding: 1px 6px; border-radius: 4px; background: rgba(127,127,127,0.18); }
.card-stats { display: grid; grid-template-columns: 1fr 1fr; gap: 8px 16px; margin: 14px 0 0; padding: 0; }
.stat { display: contents; }
.stat dt { font-size: 11px; opacity: 0.6; text-transform: uppercase; letter-spacing: 0.04em; }
.stat dd { margin: 0; font-variant-numeric: tabular-nums; font-weight: 500; }
.card-footer { margin-top: 12px; font-size: 11px; opacity: 0.6; display: grid; gap: 2px; }
.card-actions { margin-top: 12px; display: flex; gap: 6px; flex-wrap: wrap; }
.card-actions:empty { display: none; }
</style>
"#;

// ============================================================================
// Agent card adapter
// ============================================================================

/// Optional aggregate stats fetched alongside the agent. Pass zeros when
/// the storage backend cannot serve the value (e.g. tests).
#[derive(Debug, Clone, Copy, Default)]
pub struct AgentCardStats {
    pub session_count: u64,
}

/// Build a card from an `Agent` + side-loaded stats.
pub fn agent_card(agent: &Agent, stats: AgentCardStats) -> EntityCard {
    let title = agent
        .display_name
        .clone()
        .unwrap_or_else(|| agent.name.clone());
    let subtitle = if agent.display_name.is_some() {
        Some(agent.name.clone())
    } else {
        None
    };

    let mut stat_items: Vec<StatItem> = Vec::new();

    if let Some(usage) = &agent.usage {
        stat_items.push(StatItem {
            label: "Input tokens".into(),
            value: format_count(u64::from(usage.input_tokens)),
        });
        stat_items.push(StatItem {
            label: "Output tokens".into(),
            value: format_count(u64::from(usage.output_tokens)),
        });
        if let Some(cache_read) = usage.cache_read_tokens {
            stat_items.push(StatItem {
                label: "Cache read".into(),
                value: format_count(u64::from(cache_read)),
            });
        }
        if let Some(cache_create) = usage.cache_creation_tokens {
            stat_items.push(StatItem {
                label: "Cache write".into(),
                value: format_count(u64::from(cache_create)),
            });
        }
    }

    stat_items.push(StatItem {
        label: "Sessions".into(),
        value: format_count(stats.session_count),
    });

    if !agent.capabilities.is_empty() {
        stat_items.push(StatItem {
            label: "Capabilities".into(),
            value: agent.capabilities.len().to_string(),
        });
    }

    let mut footer_lines = Vec::new();
    footer_lines.push(format!("Created {}", format_date(&agent.created_at)));
    if agent.updated_at != agent.created_at {
        footer_lines.push(format!("Updated {}", format_date(&agent.updated_at)));
    }
    if let Some(archived) = agent.archived_at {
        footer_lines.push(format!("Archived {}", format_date(&archived)));
    }

    EntityCard {
        kind: EntityKind::Agent,
        public_id: agent.public_id.to_string(),
        title,
        subtitle,
        description: agent.description.clone(),
        status: agent.status.to_string(),
        tags: agent.tags.clone(),
        stats: stat_items,
        footer_lines,
    }
}

/// Short plain-text summary for hosts that don't render the embedded
/// resource. Mirrors what the card surfaces visually.
pub fn agent_card_summary(agent: &Agent, stats: AgentCardStats) -> String {
    let display = agent.display_name.as_deref().unwrap_or(agent.name.as_str());
    let usage = agent
        .usage
        .as_ref()
        .map(|u| u.input_tokens + u.output_tokens)
        .unwrap_or(0);
    format!(
        "Agent {} ({}): {} session(s), {} total tokens.",
        display,
        agent.status,
        stats.session_count,
        format_count(u64::from(usage))
    )
}

fn format_count(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*byte as char);
    }
    out
}

fn format_date(ts: &DateTime<Utc>) -> String {
    ts.format("%b %d, %Y").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use everruns_core::events::TokenUsage;
    use everruns_platform::{Agent, AgentStatus};
    use everruns_provider::typed_id::AgentId;

    fn sample_agent() -> Agent {
        Agent {
            public_id: AgentId::from_seed(1),
            internal_id: uuid::Uuid::nil(),
            name: "customer-support".into(),
            display_name: Some("Customer Support <Bot>".into()),
            description: Some("Answers FAQs.\n<script>x</script>".into()),
            system_prompt: "you help".into(),
            default_model_id: None,
            harness_id: everruns_provider::typed_id::HarnessId::from_uuid(uuid::Uuid::nil()),
            default_version_id: None,
            forked_from_agent_id: None,
            forked_from_version_id: None,
            root_agent_id: None,
            tags: vec!["faq".into(), "tier-1".into()],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            tools: vec![],
            mcp_servers: Default::default(),
            status: AgentStatus::Active,
            created_at: chrono::Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap(),
            updated_at: chrono::Utc.with_ymd_and_hms(2026, 1, 5, 0, 0, 0).unwrap(),
            archived_at: None,
            deleted_at: None,
            usage: Some(TokenUsage::with_cache(
                1_234_567,
                89_012,
                Some(50_000),
                None,
            )),
        }
    }

    #[test]
    fn escapes_all_html_specials() {
        let s = escape_html("<a href=\"x&y\">'foo'</a>");
        assert_eq!(
            s,
            "&lt;a href=&quot;x&amp;y&quot;&gt;&#x27;foo&#x27;&lt;/a&gt;"
        );
    }

    #[test]
    fn card_uri_uses_kebab_singular() {
        assert_eq!(
            card_uri(EntityKind::Agent, "agent_01"),
            "ui://everruns/agent/agent_01/card"
        );
    }

    #[test]
    fn agent_card_renders_and_escapes() {
        let agent = sample_agent();
        let card = agent_card(&agent, AgentCardStats { session_count: 7 });
        let html = render_html(&card).expect("renders");

        assert!(html.contains("Customer Support &lt;Bot&gt;"));
        // raw `<script>` from the description must not appear:
        assert!(!html.contains("<script>x</script>"));
        assert!(html.contains("&lt;script&gt;x&lt;/script&gt;"));
        assert!(html.contains("Sessions"));
        assert!(html.contains("1,234,567"));
        // The card body itself never embeds a `ui://` URI — that lives only
        // on the wrapping resource envelope.
        assert!(!html.contains("ui://"), "URI leaked into card body");
        assert!(html.contains("status-active"));
        assert!(html.contains("Content-Security-Policy"));
    }

    #[test]
    fn render_caps_size() {
        let mut card = EntityCard {
            kind: EntityKind::Agent,
            public_id: "agent_01".into(),
            title: "x".into(),
            subtitle: None,
            description: Some("a".repeat(MAX_CARD_BYTES + 100)),
            status: "active".into(),
            tags: vec![],
            stats: vec![],
            footer_lines: vec![],
        };
        assert!(render_html(&card).is_none());
        card.description = Some("ok".into());
        assert!(render_html(&card).is_some());
    }

    #[test]
    fn tool_content_shape() {
        let agent = sample_agent();
        let card = agent_card(&agent, AgentCardStats { session_count: 0 });
        let summary = agent_card_summary(&agent, AgentCardStats { session_count: 0 });
        let value = card_tool_content(&card, &summary).unwrap();
        let arr = value.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["type"], "resource");
        assert!(
            arr[0]["resource"]["uri"]
                .as_str()
                .unwrap()
                .starts_with("ui://everruns/agent/")
        );
        assert_eq!(arr[0]["resource"]["mimeType"], "text/html");
        assert_eq!(arr[1]["type"], "text");
        assert!(arr[1]["text"].as_str().unwrap().contains("Customer"));
    }
}
