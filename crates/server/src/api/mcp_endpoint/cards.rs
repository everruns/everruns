// MCP entity card data for the static MCP Apps template.
// Agent tools send structured card data to the static MCP Apps template;
// see knowledge/ui/mcp-apps.md. Serialized card data has a 64 KiB cap.

use crate::domains::agents::record::Agent;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

/// Hard cap on serialized card data — reject rather than truncate.
pub const MAX_CARD_BYTES: usize = 64 * 1024;

/// One stat row in the card body.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StatItem {
    pub label: String,
    pub value: String,
}

/// Presentation data consumed by the MCP Apps template.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EntityCard {
    /// External prefixed id (e.g. `agent_…`), shown verbatim on the card.
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
    /// Avatar as a `data:image/png;base64,…` URI. Inlined because the card's
    /// CSP blocks every network fetch (`img-src data:`).
    pub avatar_data_uri: Option<String>,
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

/// Tool results carry data for the static MCP Apps template, never embedded HTML.
pub fn card_tool_content(card: &EntityCard, summary: &str) -> Result<Value, String> {
    let data = json!({ "agent_card": card });
    if serde_json::to_vec(&data).map_err(|e| e.to_string())?.len() > MAX_CARD_BYTES {
        return Err(format!(
            "Card data exceeded {} byte cap for {}",
            MAX_CARD_BYTES, card.public_id
        ));
    }
    Ok(json!({
        "content": [{ "type": "text", "text": summary }],
        "structuredContent": data,
    }))
}

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
        public_id: agent.public_id.to_string(),
        title,
        subtitle,
        description: agent.description.clone(),
        status: agent.status.to_string(),
        tags: agent.tags.clone(),
        stats: stat_items,
        footer_lines,
        avatar_data_uri: None,
    }
}

/// [`agent_card`] with the agent's avatar inlined. A missing or unreadable
/// avatar leaves the card without one rather than failing it.
pub async fn agent_card_with_avatar(
    db: &crate::storage::StorageBackend,
    agent: &Agent,
    stats: AgentCardStats,
) -> EntityCard {
    use crate::domains::agents::avatar::{AvatarShape, variant_name};
    use base64::Engine as _;
    let mut card = agent_card(agent, stats);
    if let Some(avatar) = &agent.avatar
        && let Ok(Some(row)) = db
            .get_agent_avatar_variant(avatar.id.uuid(), &variant_name(AvatarShape::Square, 64))
            .await
    {
        let encoded = base64::engine::general_purpose::STANDARD.encode(&row.data);
        card.avatar_data_uri = Some(format!("data:{};base64,{encoded}", row.content_type));
    }
    card
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
    use crate::domains::agents::record::{Agent, AgentStatus};
    use chrono::TimeZone;
    use everruns_contracts::typed_id::AgentId;
    use everruns_core::events::TokenUsage;

    fn sample_agent() -> Agent {
        Agent {
            model_role: None,
            completion_check: Default::default(),
            localizations: Default::default(),
            is_built_in: false,
            avatar: None,
            service_virtual_user_id: None,

            public_id: AgentId::from_seed(1),
            internal_id: uuid::Uuid::nil(),
            name: "customer-support".into(),
            display_name: Some("Customer Support <Bot>".into()),
            description: Some("Answers FAQs.\n<script>x</script>".into()),
            intro_markdown: None,
            short_description: None,
            starters: Vec::new(),
            system_prompt: "you help".into(),
            default_model_id: None,
            harness_id: everruns_contracts::typed_id::HarnessId::from_uuid(uuid::Uuid::nil()),
            forked_from_agent_id: None,
            root_agent_id: None,
            owner_agent_id: None,
            tags: vec!["faq".into(), "tier-1".into()],
            capabilities: vec![],
            sandbox_policy: None,
            initial_files: vec![],
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            max_output_tokens: None,
            communication: Default::default(),
            tools: vec![],
            mcp_servers: Default::default(),
            status: AgentStatus::Active,
            exposures_suspended: false,
            exposed: false,
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
    fn agent_card_preserves_text_and_formats_stats() {
        let card = agent_card(&sample_agent(), AgentCardStats { session_count: 7 });
        assert_eq!(card.title, "Customer Support <Bot>");
        assert!(
            card.description
                .as_deref()
                .unwrap()
                .contains("<script>x</script>")
        );
        assert!(
            card.stats
                .iter()
                .any(|stat| stat.label == "Sessions" && stat.value == "7")
        );
        assert!(card.stats.iter().any(|stat| stat.value == "1,234,567"));
    }

    #[test]
    fn tool_content_shape() {
        let agent = sample_agent();
        let card = agent_card(&agent, AgentCardStats { session_count: 0 });
        let summary = agent_card_summary(&agent, AgentCardStats { session_count: 0 });
        let value = card_tool_content(&card, &summary).unwrap();
        assert_eq!(value["content"].as_array().unwrap().len(), 1);
        assert_eq!(value["content"][0]["type"], "text");
        assert!(
            value["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Customer")
        );
        assert_eq!(
            value["structuredContent"]["agent_card"]["public_id"],
            card.public_id
        );
        assert_eq!(
            value["structuredContent"]["agent_card"]["title"],
            card.title
        );
        assert_eq!(
            value["structuredContent"]["agent_card"]["stats"][0]["label"],
            card.stats[0].label
        );
        let mut oversized = card;
        oversized.description = Some("a".repeat(MAX_CARD_BYTES + 1));
        assert!(card_tool_content(&oversized, &summary).is_err());
    }
}
