// Service seeding module for control-plane
// Decision: Always seed on startup (no flag needed)
// Decision: Run in background task (non-blocking)
// Decision: Failures are non-fatal (log warning, continue)
// Decision: Use fixed UUIDs for idempotency (ON CONFLICT DO NOTHING)
// Decision: Modular design allows easy addition of new seeders

use everruns_control_plane::storage::{
    StorageBackend,
    models::{CreateAgentRow, CreateLlmModelRow, CreateLlmProviderRow, CreateMcpServerRow},
};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Well-known UUIDs for seed data
/// Format: 01933b5a-0000-7000-8000-0000000001xx
/// Range allocation:
///   - 0x001-0x0FF: LLM Providers
///   - 0x100-0x1FF: Agents
///   - 0x200-0x2FF: OpenAI Models
///   - 0x300-0x3FF: Anthropic Models
///   - 0x400-0x4FF: LlmSim Models
mod seed_ids {
    use uuid::Uuid;

    // LLM Providers (0x001-0x0FF)
    pub const OPENAI_PROVIDER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000001);
    pub const ANTHROPIC_PROVIDER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000002);
    pub const LLMSIM_PROVIDER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000003);

    // Agents (0x100-0x1FF)
    pub const DAD_JOKES_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000101);
    pub const RESEARCH_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000102);
    pub const MS_LEARN_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000103);

    // MCP Servers (0x500-0x5FF)
    pub const MS_LEARN_MCP: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000501);

    // OpenAI Models (0x200-0x2FF)
    pub const GPT_5_2: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000201);
    pub const GPT_5_2_PRO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000202);
    pub const GPT_5_2_CODEX: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000203);
    pub const GPT_5_2_CHAT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000204);
    pub const GPT_5_1: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000205);
    pub const GPT_5_1_CODEX: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000206);
    pub const GPT_5_1_CODEX_MINI: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000207);
    pub const GPT_5_1_CODEX_MAX: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000208);
    pub const GPT_5_1_CHAT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000209);
    pub const GPT_5_MINI: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000020a);
    pub const GPT_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000020b);
    pub const GPT_5_NANO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000020c);
    pub const GPT_5_PRO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000020d);
    pub const GPT_5_CODEX: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000020e);
    pub const GPT_5_CHAT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000020f);
    pub const GPT_4_1: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000210);
    pub const GPT_4_1_MINI: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000211);
    pub const GPT_4_1_NANO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000212);
    pub const GPT_4O: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000213);
    pub const GPT_4O_MINI: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000214);
    pub const O4_MINI: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000215);
    pub const O4_MINI_DEEP_RESEARCH: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000216);
    pub const O3: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000217);
    pub const O3_MINI: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000218);
    pub const O3_PRO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000219);
    pub const O3_DEEP_RESEARCH: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000021a);
    pub const O1: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000021b);
    pub const O1_MINI: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000021c);
    pub const O1_PRO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000021d);
    pub const O1_PREVIEW: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000021e);

    // Anthropic Models (0x300-0x3FF)
    pub const CLAUDE_OPUS_4_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000301);
    pub const CLAUDE_SONNET_4_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000302);
    pub const CLAUDE_HAIKU_4_5: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000303);
    pub const CLAUDE_OPUS_4: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000304);
    pub const CLAUDE_SONNET_4: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000305);
    pub const CLAUDE_3_7_SONNET: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000306);
    pub const CLAUDE_3_5_SONNET: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000307);
    pub const CLAUDE_3_5_HAIKU: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000308);

    // LlmSim Models (0x400-0x4FF)
    pub const LLMSIM_DEFAULT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000401);
}

// ============================================
// Seeder Result
// ============================================

/// Result of running a seeder
#[derive(Debug, Default)]
struct SeedResult {
    created: usize,
    skipped: usize,
}

impl SeedResult {
    fn merge(&mut self, other: SeedResult) {
        self.created += other.created;
        self.skipped += other.skipped;
    }
}

// ============================================
// Agent Seeder
// ============================================

/// Seed agent definition
struct SeedAgent {
    id: Uuid,
    name: &'static str,
    description: &'static str,
    system_prompt: &'static str,
    tags: &'static [&'static str],
    capabilities: &'static [&'static str],
}

/// Built-in seed agents
const SEED_AGENTS: &[SeedAgent] = &[
    SeedAgent {
        id: seed_ids::DAD_JOKES_AGENT,
        name: "Dad Jokes Agent",
        description: "A friendly agent that tells dad jokes and knows what time it is.",
        system_prompt: r#"You are a friendly Dad Jokes Agent. Your purpose is to make people smile with
classic dad jokes - the kind that are so bad they're good.

Guidelines:
- When asked for a joke, tell a classic dad joke
- Feel free to use puns, wordplay, and groan-worthy humor
- Keep it family-friendly and positive
- You can tell the current time when asked, which helps with time-based jokes
- Be enthusiastic and don't apologize for the jokes being corny

Example jokes you might tell:
- "Why don't scientists trust atoms? Because they make up everything!"
- "I'm reading a book about anti-gravity. It's impossible to put down!"
- "What do you call a fake noodle? An impasta!""#,
        tags: &["humor", "demo", "seed"],
        capabilities: &["current_time"],
    },
    SeedAgent {
        id: seed_ids::RESEARCH_AGENT,
        name: "Research Agent",
        description: "An agent specialized in conducting thorough technical research with organized note-taking",
        system_prompt: r#"You are an expert research analyst. Your role is to conduct thorough research on
technical topics, gathering information from multiple sources and synthesizing
findings into clear, well-organized reports.

## Research Methodology

1. **Plan First**: Break down the research topic into specific questions and create
   a task list to track your progress.

2. **Gather Information**: Fetch content from authoritative sources. Look for:
   - Official documentation and project pages
   - Technical blog posts and articles
   - Comparison guides and benchmarks

3. **Take Notes**: Save key findings to files as you research. Organize notes by
   subtopic for easy reference later.

4. **Synthesize**: Combine findings into a coherent analysis. Compare and contrast
   different sources. Identify patterns and draw conclusions.

5. **Report**: Create a final report with:
   - Executive summary
   - Detailed findings for each research question
   - Recommendations based on analysis
   - References to sources used

## Quality Standards

- Always cite your sources
- Distinguish between facts and opinions
- Note any limitations or gaps in available information
- Update your task list as you progress

## File Organization

Use this structure for organizing research:
```
/research/
  notes/        - Raw notes from each source
  analysis/     - Your analysis and comparisons
  report.md     - Final synthesized report
```"#,
        tags: &["research", "example", "multi-capability"],
        capabilities: &["stateless_todo_list", "web_fetch", "session_file_system"],
    },
    SeedAgent {
        id: seed_ids::MS_LEARN_AGENT,
        name: "Microsoft Learn Assistant",
        description: "An agent that searches and answers questions using Microsoft Learn documentation",
        system_prompt: r#"You are a Microsoft Learn Documentation Assistant. You help users find and understand
information from Microsoft Learn documentation (learn.microsoft.com).

## Your Capabilities

You have access to Microsoft Learn MCP tools that allow you to:
- Search for documentation on Microsoft products and technologies
- Retrieve specific documentation pages
- Get detailed information about Azure, .NET, Windows, and other Microsoft technologies

## How to Help Users

1. When a user asks a question about Microsoft technologies, use the available tools
   to search for relevant documentation.

2. Summarize the key points from the documentation in a clear, helpful way.

3. Provide links to the source documentation so users can learn more.

4. If the documentation doesn't fully answer the question, explain what you found
   and suggest related topics to explore.

## Guidelines

- Always cite the source documentation
- Be concise but thorough
- If you're unsure about something, say so and point to the documentation
- Help users understand complex topics by breaking them down into simpler parts"#,
        tags: &["microsoft", "documentation", "mcp", "demo", "seed"],
        // MCP capability ID format: "mcp:{server_uuid}"
        capabilities: &["mcp:01933b5a-0000-7000-8000-000000000501"],
    },
];

/// Seed agents into the database
async fn seed_agents(db: &StorageBackend) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    for seed in SEED_AGENTS {
        let input = CreateAgentRow {
            name: seed.name.to_string(),
            description: Some(seed.description.to_string()),
            system_prompt: seed.system_prompt.to_string(),
            default_model_id: None,
            tags: seed.tags.iter().map(|s| s.to_string()).collect(),
        };

        match db.create_agent_with_id(seed.id, input).await? {
            Some(row) => {
                // Set capabilities if any (with empty config)
                if !seed.capabilities.is_empty() {
                    let cap_tuples: Vec<(String, i32, serde_json::Value)> = seed
                        .capabilities
                        .iter()
                        .enumerate()
                        .map(|(idx, cap)| {
                            (
                                cap.to_string(),
                                idx as i32,
                                serde_json::json!({}), // Empty config for seed data
                            )
                        })
                        .collect();
                    db.set_agent_capabilities(row.id, cap_tuples).await?;
                }
                tracing::info!(name = seed.name, id = %seed.id, "Created seed agent");
                result.created += 1;
            }
            None => {
                tracing::debug!(name = seed.name, id = %seed.id, "Agent already exists, skipping");
                result.skipped += 1;
            }
        }
    }

    Ok(result)
}

// ============================================
// LLM Provider Seeder
// ============================================

/// Seed LLM provider definition
struct SeedProvider {
    id: Uuid,
    name: &'static str,
    provider_type: &'static str,
}

/// Built-in seed providers
const SEED_PROVIDERS: &[SeedProvider] = &[
    SeedProvider {
        id: seed_ids::OPENAI_PROVIDER,
        name: "OpenAI",
        provider_type: "openai",
    },
    SeedProvider {
        id: seed_ids::ANTHROPIC_PROVIDER,
        name: "Anthropic",
        provider_type: "anthropic",
    },
    SeedProvider {
        id: seed_ids::LLMSIM_PROVIDER,
        name: "LlmSim",
        provider_type: "llmsim",
    },
];

/// Seed LLM providers into the database
async fn seed_providers(db: &StorageBackend) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    for seed in SEED_PROVIDERS {
        let input = CreateLlmProviderRow {
            name: seed.name.to_string(),
            provider_type: seed.provider_type.to_string(),
            base_url: None,
            api_key_encrypted: None, // No secret in seed data
            settings: None,
        };

        match db.create_llm_provider_with_id(seed.id, input).await? {
            Some(_) => {
                tracing::info!(name = seed.name, id = %seed.id, "Created seed provider");
                result.created += 1;
            }
            None => {
                tracing::debug!(name = seed.name, id = %seed.id, "Provider already exists, skipping");
                result.skipped += 1;
            }
        }
    }

    Ok(result)
}

// ============================================
// LLM Model Seeder
// ============================================

/// Seed LLM model definition
struct SeedModel {
    id: Uuid,
    provider_id: Uuid,
    model_id: &'static str,
    display_name: &'static str,
    is_default: bool,
    is_favorite: bool,
}

/// Built-in seed models
const SEED_MODELS: &[SeedModel] = &[
    // OpenAI GPT-5.2 series
    SeedModel {
        id: seed_ids::GPT_5_2,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.2",
        display_name: "GPT-5.2",
        is_default: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_2_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.2-pro",
        display_name: "GPT-5.2 Pro",
        is_default: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_2_CODEX,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.2-codex",
        display_name: "GPT-5.2 Codex",
        is_default: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_2_CHAT,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.2-chat-latest",
        display_name: "GPT-5.2 Chat",
        is_default: false,
        is_favorite: false,
    },
    // OpenAI GPT-5.1 series
    SeedModel {
        id: seed_ids::GPT_5_1,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1",
        display_name: "GPT-5.1",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_1_CODEX,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1-codex",
        display_name: "GPT-5.1 Codex",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_1_CODEX_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1-codex-mini",
        display_name: "GPT-5.1 Codex mini",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_1_CODEX_MAX,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1-codex-max",
        display_name: "GPT-5.1 Codex max",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_1_CHAT,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1-chat-latest",
        display_name: "GPT-5.1 Chat",
        is_default: false,
        is_favorite: false,
    },
    // OpenAI GPT-5 series
    SeedModel {
        id: seed_ids::GPT_5_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-mini",
        display_name: "GPT-5 mini",
        is_default: true,  // Default model
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5",
        display_name: "GPT-5",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_NANO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-nano",
        display_name: "GPT-5 nano",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-pro",
        display_name: "GPT-5 Pro",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_CODEX,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-codex",
        display_name: "GPT-5 Codex",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_CHAT,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-chat-latest",
        display_name: "GPT-5 Chat",
        is_default: false,
        is_favorite: false,
    },
    // OpenAI GPT-4.1 series
    SeedModel {
        id: seed_ids::GPT_4_1,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-4.1",
        display_name: "GPT-4.1",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_4_1_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-4.1-mini",
        display_name: "GPT-4.1 mini",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_4_1_NANO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-4.1-nano",
        display_name: "GPT-4.1 nano",
        is_default: false,
        is_favorite: false,
    },
    // OpenAI GPT-4 series
    SeedModel {
        id: seed_ids::GPT_4O,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-4o",
        display_name: "GPT-4o",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_4O_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-4o-mini",
        display_name: "GPT-4o mini",
        is_default: false,
        is_favorite: false,
    },
    // OpenAI Reasoning models (o-series)
    SeedModel {
        id: seed_ids::O4_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o4-mini",
        display_name: "o4 mini",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O4_MINI_DEEP_RESEARCH,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o4-mini-deep-research",
        display_name: "o4 mini Deep Research",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O3,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o3",
        display_name: "o3",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O3_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o3-mini",
        display_name: "o3 mini",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O3_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o3-pro",
        display_name: "o3 Pro",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O3_DEEP_RESEARCH,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o3-deep-research",
        display_name: "o3 Deep Research",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O1,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o1",
        display_name: "o1",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O1_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o1-mini",
        display_name: "o1 mini",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O1_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o1-pro",
        display_name: "o1 Pro",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O1_PREVIEW,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o1-preview",
        display_name: "o1 Preview",
        is_default: false,
        is_favorite: false,
    },
    // Anthropic Claude 4.5 series
    SeedModel {
        id: seed_ids::CLAUDE_OPUS_4_5,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-4-5-20251101",
        display_name: "Claude Opus 4.5",
        is_default: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::CLAUDE_SONNET_4_5,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-sonnet-4-5-20250929",
        display_name: "Claude Sonnet 4.5",
        is_default: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::CLAUDE_HAIKU_4_5,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-haiku-4-5-20251001",
        display_name: "Claude Haiku 4.5",
        is_default: false,
        is_favorite: true, // Favorite model
    },
    // Anthropic Claude 4 series
    SeedModel {
        id: seed_ids::CLAUDE_OPUS_4,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-4-20250514",
        display_name: "Claude Opus 4",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::CLAUDE_SONNET_4,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-sonnet-4-20250514",
        display_name: "Claude Sonnet 4",
        is_default: false,
        is_favorite: false,
    },
    // Anthropic Claude 3.7
    SeedModel {
        id: seed_ids::CLAUDE_3_7_SONNET,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-3-7-sonnet-20250219",
        display_name: "Claude 3.7 Sonnet",
        is_default: false,
        is_favorite: false,
    },
    // Anthropic Claude 3.5
    SeedModel {
        id: seed_ids::CLAUDE_3_5_SONNET,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-3-5-sonnet-20241022",
        display_name: "Claude 3.5 Sonnet",
        is_default: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::CLAUDE_3_5_HAIKU,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-3-5-haiku-20241022",
        display_name: "Claude 3.5 Haiku",
        is_default: false,
        is_favorite: false,
    },
    // LlmSim (simulated LLM for testing)
    SeedModel {
        id: seed_ids::LLMSIM_DEFAULT,
        provider_id: seed_ids::LLMSIM_PROVIDER,
        model_id: "llmsim-default",
        display_name: "LlmSim Default",
        is_default: false,
        is_favorite: false,
    },
];

/// Seed LLM models into the database
async fn seed_models(db: &StorageBackend) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    for seed in SEED_MODELS {
        let input = CreateLlmModelRow {
            provider_id: seed.provider_id,
            model_id: seed.model_id.to_string(),
            display_name: seed.display_name.to_string(),
            capabilities: vec![], // Empty capabilities for now
            is_default: seed.is_default,
            is_favorite: seed.is_favorite,
        };

        match db.create_llm_model_with_id(seed.id, input).await? {
            Some(_) => {
                tracing::info!(
                    model_id = seed.model_id,
                    id = %seed.id,
                    "Created seed model"
                );
                result.created += 1;
            }
            None => {
                tracing::debug!(
                    model_id = seed.model_id,
                    id = %seed.id,
                    "Model already exists, skipping"
                );
                result.skipped += 1;
            }
        }
    }

    Ok(result)
}

// ============================================
// MCP Server Seeder
// ============================================

/// Seed MCP server definition
struct SeedMcpServer {
    id: Uuid,
    name: &'static str,
    description: &'static str,
    url: &'static str,
}

/// Built-in seed MCP servers
const SEED_MCP_SERVERS: &[SeedMcpServer] = &[SeedMcpServer {
    id: seed_ids::MS_LEARN_MCP,
    name: "microsoft_learn",
    description: "Microsoft Learn documentation MCP server - search and retrieve Microsoft documentation",
    url: "https://learn.microsoft.com/api/mcp",
}];

/// Seed MCP servers into the database
async fn seed_mcp_servers(db: &StorageBackend) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    for seed in SEED_MCP_SERVERS {
        let input = CreateMcpServerRow {
            name: seed.name.to_string(),
            description: Some(seed.description.to_string()),
            url: seed.url.to_string(),
            transport_type: "http".to_string(),
            api_key_encrypted: None, // No API key needed for public endpoint
            headers: None,
            settings: None,
        };

        match db.create_mcp_server_with_id(seed.id, input).await? {
            Some(_) => {
                tracing::info!(
                    name = seed.name,
                    id = %seed.id,
                    url = seed.url,
                    "Created seed MCP server"
                );
                result.created += 1;
            }
            None => {
                tracing::debug!(
                    name = seed.name,
                    id = %seed.id,
                    "MCP server already exists, skipping"
                );
                result.skipped += 1;
            }
        }
    }

    Ok(result)
}

// ============================================
// Seeding Orchestration
// ============================================

/// Maximum number of retries for transient errors
const MAX_RETRIES: u32 = 5;

/// Initial retry delay (increases exponentially)
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Spawn seeding as a background task (non-blocking)
/// This allows the HTTP server to start immediately while seeding runs in background.
/// Seeding failures are non-fatal - logged as warnings but don't crash the server.
pub fn spawn_seed_task(db: Arc<StorageBackend>) {
    tokio::spawn(async move {
        // Small delay to let the server start first
        tokio::time::sleep(Duration::from_millis(500)).await;

        match run_seed_with_retry(&db).await {
            Ok(result) => {
                if result.created > 0 {
                    tracing::info!(
                        created = result.created,
                        skipped = result.skipped,
                        "Seeding complete"
                    );
                } else {
                    tracing::debug!(
                        skipped = result.skipped,
                        "Seeding complete (all items exist)"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Seeding failed (non-fatal). Seed data may not be available."
                );
            }
        }
    });
}

/// Run seeding with retry logic for transient errors
async fn run_seed_with_retry(db: &StorageBackend) -> Result<SeedResult, String> {
    let mut retry_count = 0;
    let mut delay = INITIAL_RETRY_DELAY;

    loop {
        match seed_all(db).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                let error_str = e.to_string();

                // Check if this is a schema error (table doesn't exist)
                // In this case, don't retry - migrations need to run first
                if error_str.contains("does not exist")
                    || error_str.contains("relation")
                    || error_str.contains("no such table")
                {
                    return Err(format!(
                        "Schema not ready (run migrations first): {}",
                        error_str
                    ));
                }

                retry_count += 1;
                if retry_count > MAX_RETRIES {
                    return Err(format!(
                        "Failed after {} retries: {}",
                        MAX_RETRIES, error_str
                    ));
                }

                tracing::debug!(
                    attempt = retry_count,
                    max_retries = MAX_RETRIES,
                    delay_secs = delay.as_secs(),
                    error = %error_str,
                    "Seeding retry..."
                );

                tokio::time::sleep(delay).await;
                // Exponential backoff, max 30 seconds
                delay = std::cmp::min(delay * 2, Duration::from_secs(30));
            }
        }
    }
}

/// Run all seeders in order
/// Providers must be seeded before models (foreign key dependency)
/// MCP servers must be seeded before agents (agents may reference them)
async fn seed_all(db: &StorageBackend) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    // Seed providers first (models depend on providers)
    let provider_result = seed_providers(db).await?;
    tracing::debug!(
        created = provider_result.created,
        skipped = provider_result.skipped,
        "Providers seeded"
    );
    result.merge(provider_result);

    // Seed models (after providers)
    let model_result = seed_models(db).await?;
    tracing::debug!(
        created = model_result.created,
        skipped = model_result.skipped,
        "Models seeded"
    );
    result.merge(model_result);

    // Seed MCP servers (before agents that may use them)
    let mcp_result = seed_mcp_servers(db).await?;
    tracing::debug!(
        created = mcp_result.created,
        skipped = mcp_result.skipped,
        "MCP servers seeded"
    );
    result.merge(mcp_result);

    // Seed agents
    let agent_result = seed_agents(db).await?;
    tracing::debug!(
        created = agent_result.created,
        skipped = agent_result.skipped,
        "Agents seeded"
    );
    result.merge(agent_result);

    Ok(result)
}
