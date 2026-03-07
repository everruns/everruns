// Service seeding module for server (control plane)
// Decision: Always seed on startup (no flag needed)
// Decision: Run in background task (non-blocking)
// Decision: Failures are non-fatal (log warning, continue)
// Decision: Use fixed UUIDs for idempotency
// Decision: Upsert seed data on conflict, only when values changed (ON CONFLICT DO UPDATE ... WHERE differs)
// Decision: Modular design allows easy addition of new seeders

use crate::storage::{
    StorageBackend,
    models::{
        CreateAgentRow, CreateHarnessRow, CreateLlmModelRow, CreateLlmProviderRow,
        CreateMcpServerRow, CreateOrganizationRow, CreateUserRow,
    },
};
use everruns_core::{
    ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID, ANONYMOUS_USER_NAME, DEFAULT_ORG_ID,
    DEFAULT_ORG_PUBLIC_ID, DeploymentGrade,
};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Well-known UUID for the Chat harness (used by global chat endpoint)
pub const CHAT_HARNESS_ID: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000603);

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
    pub const GEMINI_PROVIDER: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000004);

    // Agents (0x100-0x1FF)
    pub const DAD_JOKES_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000101);
    pub const RESEARCH_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000102);
    pub const MS_LEARN_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000103);
    pub const PYTHON_CODER_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000104);
    pub const SHELL_ASSISTANT_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000105);
    pub const DATA_ANALYST_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000106);
    pub const CODESANDBOX_CODER_AGENT: Uuid =
        Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000107);
    pub const DAYTONA_CODER_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000108);
    pub const CLOUD_INFRA_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000109);
    pub const PLATFORM_MANAGER_AGENT: Uuid =
        Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000010a);

    // MCP Servers (0x500-0x5FF)
    pub const MS_LEARN_MCP: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000501);

    // Harnesses (0x600-0x6FF)
    pub const BASE_HARNESS: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000601);
    pub const GENERIC_HARNESS: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000602);
    pub const CHAT_HARNESS: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000603);

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
    pub const CLAUDE_OPUS_4_6: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000309);
    pub const CLAUDE_SONNET_4_6: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000030a);
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
    pub const LLMSIM_LATENCY: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000402);

    // Gemini Models (0x600-0x6FF)
    pub const GEMINI_25_PRO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000601);
    pub const GEMINI_25_FLASH: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000602);
    pub const GEMINI_20_FLASH: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000603);
}

// ============================================
// Seeder Result
// ============================================

/// Result of running a seeder
#[derive(Debug, Default)]
pub struct SeedResult {
    created: usize,
    updated: usize,
    unchanged: usize,
}

impl SeedResult {
    fn merge(&mut self, other: SeedResult) {
        self.created += other.created;
        self.updated += other.updated;
        self.unchanged += other.unchanged;
    }

    fn has_changes(&self) -> bool {
        self.created > 0 || self.updated > 0
    }
}

/// Sync capabilities for an agent, only writing if the set actually changed.
/// Returns true if capabilities were updated.
async fn sync_agent_capabilities(
    db: &StorageBackend,
    agent_id: Uuid,
    desired: &[&str],
) -> anyhow::Result<bool> {
    let current = db.get_agent_capabilities(agent_id).await?;
    let current_ids: Vec<&str> = current.iter().map(|c| c.capability_id.as_str()).collect();
    let desired_ids: Vec<&str> = desired.to_vec();

    if current_ids == desired_ids {
        return Ok(false);
    }

    let cap_tuples: Vec<(String, i32, serde_json::Value)> = desired
        .iter()
        .enumerate()
        .map(|(idx, cap)| (cap.to_string(), idx as i32, serde_json::json!({})))
        .collect();
    db.set_agent_capabilities(agent_id, cap_tuples).await?;
    Ok(true)
}

/// Sync capabilities for a harness, only writing if the set actually changed.
/// Returns true if capabilities were updated.
async fn sync_harness_capabilities(
    db: &StorageBackend,
    harness_id: Uuid,
    desired: &[&str],
) -> anyhow::Result<bool> {
    let current = db.get_harness_capabilities(harness_id).await?;
    let current_ids: Vec<&str> = current.iter().map(|c| c.capability_id.as_str()).collect();
    let desired_ids: Vec<&str> = desired.to_vec();

    if current_ids == desired_ids {
        return Ok(false);
    }

    let cap_tuples: Vec<(String, i32, serde_json::Value)> = desired
        .iter()
        .enumerate()
        .map(|(idx, cap)| (cap.to_string(), idx as i32, serde_json::json!({})))
        .collect();
    db.set_harness_capabilities(harness_id, cap_tuples).await?;
    Ok(true)
}

// ============================================
// Default Organization Seeder
// ============================================

/// Seed the default organization (must run first).
/// Orgs use DO NOTHING since their seed data is static.
async fn seed_default_organization(db: &StorageBackend) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    let input = CreateOrganizationRow {
        public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
        name: "Default Organization".to_string(),
        created_by: None,
    };

    match db
        .create_organization_with_id(DEFAULT_ORG_ID, input)
        .await?
    {
        Some(_) => {
            tracing::info!(
                org_id = DEFAULT_ORG_ID,
                public_id = DEFAULT_ORG_PUBLIC_ID,
                "Created default organization"
            );
            result.created += 1;
        }
        None => {
            tracing::debug!(
                org_id = DEFAULT_ORG_ID,
                public_id = DEFAULT_ORG_PUBLIC_ID,
                "Default organization up to date"
            );
            result.unchanged += 1;
        }
    }

    Ok(result)
}

// ============================================
// Anonymous User Seeder
// ============================================

/// Seed anonymous user for auth=none mode.
/// Uses ANONYMOUS_USER_ID so all code paths (org membership, API keys, etc.)
/// work without special-casing a nil/missing user.
async fn seed_anonymous_user(db: &StorageBackend) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    let input = CreateUserRow {
        email: ANONYMOUS_USER_EMAIL.to_string(),
        name: ANONYMOUS_USER_NAME.to_string(),
        avatar_url: None,
        roles: vec!["admin".to_string()],
        password_hash: None,
        email_verified: true,
        auth_provider: Some("none".to_string()),
        auth_provider_id: None,
        external_id: None,
    };

    match db.create_user_with_id(ANONYMOUS_USER_ID, input).await? {
        Some(_) => {
            tracing::info!("Created anonymous user");
            result.created += 1;
        }
        None => {
            tracing::debug!("Anonymous user up to date");
            result.unchanged += 1;
        }
    }

    // Ensure anonymous user is owner of default org
    db.ensure_membership(ANONYMOUS_USER_ID, DEFAULT_ORG_ID, "owner")
        .await?;

    Ok(result)
}

// ============================================
// Harness Seeder
// ============================================

/// Seed harness definition
struct SeedHarness {
    id: Uuid,
    name: &'static str,
    description: &'static str,
    system_prompt: &'static str,
    tags: &'static [&'static str],
    capabilities: &'static [&'static str],
}

/// Built-in seed harnesses
const SEED_HARNESSES: &[SeedHarness] = &[
    SeedHarness {
        id: seed_ids::BASE_HARNESS,
        name: "Base",
        description: "Empty harness with no capabilities. Provides a blank canvas for custom configurations.",
        system_prompt: "You are a helpful assistant.",
        tags: &["base", "seed"],
        capabilities: &[],
    },
    SeedHarness {
        id: seed_ids::GENERIC_HARNESS,
        name: "Generic",
        description: "General-purpose harness with file system, bash, secrets, session management, and agent skills. Recommended default for most use cases.",
        system_prompt: "You are a helpful assistant.",
        tags: &["generic", "default", "seed"],
        capabilities: &[
            "session_file_system",
            "virtual_bash",
            "session_storage",
            "session",
            "agent_instructions",
            "skills",
            "openai_tool_search",
        ],
    },
    SeedHarness {
        id: seed_ids::CHAT_HARNESS,
        name: "Platform Chat",
        description: "Conversational harness for the global chat interface with platform management capabilities.",
        system_prompt: "You are a helpful assistant on the Everruns platform.\n\nCapabilities are the primary way to extend agent functionality. Use `list_capabilities` to discover available capabilities (built-in, MCP servers, and skills), then assign them when creating agents or harnesses.\n\nWhen creating agents, always use `list_capabilities` first to find relevant capability IDs to include.\n\n## Running agents\n\nWhen asked to \"run an agent\" or \"run X with agent Y\", follow these steps:\n1. Create a session for the agent (use `manage_sessions` with operation \"create\")\n2. Send the user's message/task to the session (use `session_interact` with operation \"send_message\")\n3. Wait for the turn to complete (use `session_interact` with operation \"wait_for_idle\")\n4. Retrieve and relay the results (use `session_interact` with operation \"get_messages\")\n\n## Harness creation\n\nAvoid creating new harnesses unless the user explicitly needs a custom one. For most tasks, use the built-in \"Generic\" harness (find it via `manage_harnesses` with operation \"list\") which already includes file system, bash, storage, session, agent instructions, and skills capabilities.\n\n## Confirmation guidelines\n\n- **Always confirm** before creating a harness or agent — these are reusable org-wide entities.\n- **Sessions**: Use common sense. Routine requests (\"run agent X on this task\") can proceed without confirmation. Unusual or high-impact requests (destructive operations, large-scale actions, unclear intent) should be confirmed first.",
        tags: &["chat", "seed"],
        capabilities: &[
            "session_file_system",
            "virtual_bash",
            "session_storage",
            "session",
            "agent_instructions",
            "skills",
            "platform_management",
            "openai_tool_search",
        ],
    },
];

/// Seed harnesses into the database (upserts, only when changed)
async fn seed_harnesses(db: &StorageBackend) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    for seed in SEED_HARNESSES {
        let input = CreateHarnessRow {
            name: seed.name.to_string(),
            description: Some(seed.description.to_string()),
            system_prompt: seed.system_prompt.to_string(),
            default_model_id: None,
            tags: seed.tags.iter().map(|s| s.to_string()).collect(),
        };

        match db
            .create_harness_with_id(DEFAULT_ORG_ID, seed.id.into(), input)
            .await?
        {
            Some(row) => {
                // Row was created or updated — sync capabilities
                sync_harness_capabilities(db, row.id.uuid(), seed.capabilities).await?;
                if row.created_at == row.updated_at {
                    tracing::info!(name = seed.name, id = %seed.id, "Created seed harness");
                    result.created += 1;
                } else {
                    tracing::info!(name = seed.name, id = %seed.id, "Updated seed harness");
                    result.updated += 1;
                }
            }
            None => {
                // Row unchanged — check if capabilities need syncing
                let caps_changed =
                    sync_harness_capabilities(db, seed.id, seed.capabilities).await?;
                if caps_changed {
                    tracing::info!(name = seed.name, id = %seed.id, "Updated seed harness capabilities");
                    result.updated += 1;
                } else {
                    tracing::debug!(name = seed.name, id = %seed.id, "Harness up to date");
                    result.unchanged += 1;
                }
            }
        }
    }

    Ok(result)
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
    /// If true, only seed in dev environments (experimental features)
    dev_only: bool,
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
        dev_only: false,
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
        dev_only: false,
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
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::PYTHON_CODER_AGENT,
        name: "Python Coder",
        description: "A fast coding agent that writes, executes, and debugs Python code in a Docker container",
        system_prompt: r#"You are a Python Coder Agent with access to a Docker container running Python.
You can write code, execute it, and iterate quickly to solve programming tasks.

## Your Capabilities

You have access to a Docker container with Python installed. You can:
- **docker_exec**: Execute shell commands including running Python scripts
- **docker_write_file**: Create or update files in the container
- **docker_read_file**: Read files from the container

## Workflow

1. **Understand the Task**: Clarify requirements before coding
2. **Write Code**: Create Python files using `docker_write_file`
3. **Execute & Test**: Run your code with `docker_exec`
4. **Iterate**: Fix errors, refine, and improve until it works

## Best Practices

- Start with a simple solution, then iterate
- Write clean, readable code with comments
- Test early and often - run code after each significant change
- Use print statements or logging for debugging
- Save your work to files so it persists across executions

## Example Usage

To write and run a Python script:
1. `docker_write_file` to `/workspace/main.py` with your code
2. `docker_exec` with `python /workspace/main.py`
3. Check output, fix issues, repeat

## Container Info

- Working directory: `/workspace`
- Python is pre-installed
- You can install packages with `pip install <package>`
- The container persists for the session - installed packages stay available

## Guidelines

- Be concise in explanations, focus on working code
- Show your reasoning when debugging
- Ask clarifying questions if the task is ambiguous
- Celebrate when things work! 🎉"#,
        tags: &["python", "coding", "docker", "demo", "seed"],
        capabilities: &["docker_container"],
        dev_only: true, // Experimental capability, only in dev environments
    },
    SeedAgent {
        id: seed_ids::SHELL_ASSISTANT_AGENT,
        name: "Shell Assistant",
        description: "An agent that helps with shell scripting and file manipulation using a sandboxed bash environment",
        system_prompt: r#"You are a Shell Assistant with access to a sandboxed bash environment.
You help users with shell scripting, text processing, and file manipulation tasks.

## Your Capabilities

You have access to:
- **bash**: Execute bash commands in an isolated virtual environment
- **session_file_system**: Read and write files that persist in the session

## What You Can Do

- Write and execute shell scripts
- Process text with tools like grep, sed, awk, and jq
- Manipulate files and directories
- Demonstrate shell scripting techniques
- Help debug shell scripts
- Explain Unix command line concepts

## Workflow

1. **Understand the Task**: Clarify what the user wants to accomplish
2. **Plan**: Break complex tasks into steps using shell commands
3. **Execute**: Run commands using the `bash` tool
4. **Verify**: Check results and iterate if needed

## Example Tasks

- "Count lines in a file" → `wc -l filename`
- "Find all .txt files" → `find . -name "*.txt"` or `ls **/*.txt`
- "Extract emails from text" → `grep -E '[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}' file`
- "Transform JSON data" → `cat data.json | jq '.items[]'`
- "Create a backup script" → Write and test a shell script

## Best Practices

- Use pipes to chain commands efficiently
- Check exit codes for error handling
- Use variables for repeated values
- Add comments to scripts for clarity
- Test commands step-by-step before combining them

## Session Filesystem

Files you create are stored in the session's virtual filesystem:
- Write files with `write_file` or shell redirections (`>`, `>>`)
- Read files with `read_file` or shell commands (`cat`, `head`, `tail`)
- The filesystem starts empty but persists throughout the session"#,
        tags: &["shell", "bash", "scripting", "demo", "seed"],
        capabilities: &["virtual_bash", "session_file_system"],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::DATA_ANALYST_AGENT,
        name: "Data Analyst",
        description: "An agent that analyzes data using SQL databases, takes notes, and produces reports",
        system_prompt: r#"You are a Data Analyst Agent. You help users analyze data by creating SQL databases,
importing data, running queries, and producing clear reports.

## Your Capabilities

You have access to:
- **sql_execute**: Create tables, insert/update/delete data. Databases are auto-created.
- **sql_query**: Run SELECT queries returning columns and rows.
- **sql_schema**: Inspect database schema (tables, columns, types).
- **session_file_system**: Save reports and notes as files.
- **stateless_todo_list**: Track analysis tasks.

## Workflow

1. **Understand the Data**: Ask what data the user wants to analyze.
2. **Design Schema**: Create appropriate tables with `sql_execute`.
3. **Import Data**: Insert the data into tables.
4. **Analyze**: Run queries to answer questions — aggregations, joins, filters.
5. **Report**: Summarize findings. Save reports to files.

## Example

```
sql_execute(database="analytics", sql="CREATE TABLE sales (id INTEGER PRIMARY KEY, product TEXT, amount REAL, date TEXT)")
sql_execute(database="analytics", sql="INSERT INTO sales VALUES (1, 'Widget', 29.99, '2024-01-15'), ...")
sql_query(database="analytics", sql="SELECT product, SUM(amount) as total FROM sales GROUP BY product ORDER BY total DESC")
```

## Guidelines

- Use descriptive table and column names
- Add PRIMARY KEY constraints
- Use appropriate types (INTEGER, REAL, TEXT)
- Results are limited to 1000 rows per query
- Databases persist for the session"#,
        tags: &["data", "sql", "analytics", "demo", "seed"],
        capabilities: &[
            "session_sql_database",
            "session_file_system",
            "stateless_todo_list",
        ],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::CODESANDBOX_CODER_AGENT,
        name: "CodeSandbox Coder",
        description: "A coding agent that runs code in cloud sandboxes powered by CodeSandbox",
        system_prompt: r#"You are a CodeSandbox Coder Agent. You run code in cloud sandbox VMs powered by CodeSandbox.

Prerequisite: The session must have CSB_API_KEY set in secrets before you can use sandbox tools.
If missing, tell the user to set it via the API or harness config.

Workflow:
1. Create sandbox: `csb_create_sandbox` (working directory: /sandbox)
2. Clone repos: `csb_git_clone` (clones to /sandbox/owner/repo)
3. Write code / install deps: `csb_write_file`, `csb_exec`
4. For long-running tasks: `csb_exec` with `wait: false`, poll `csb_exec_status`
5. Save results: `csb_download_workspace`
6. Clean up: `csb_manage_sandbox` action="shutdown"

You can run multiple sandboxes in parallel for different tasks.
Always shut down sandboxes when done."#,
        tags: &["coding", "cloud", "sandbox", "codesandbox", "demo", "seed"],
        capabilities: &["codesandbox", "session_storage", "session_file_system"],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::DAYTONA_CODER_AGENT,
        name: "Daytona Coder",
        description: "A coding agent that runs code in cloud sandboxes powered by Daytona",
        system_prompt: r#"You are a Daytona Coder Agent. You run code in cloud sandboxes powered by Daytona.

Prerequisite: The session must have DAYTONA_API_KEY set in secrets before you can use sandbox tools.
If missing, tell the user to set it via the API or harness config.

Workflow:
1. Create sandbox: `daytona_create_sandbox` (working directory: /sandbox)
2. Clone repos: `daytona_git_clone` (clones to /sandbox/owner/repo)
3. Write code / install deps: `daytona_write_file`, `daytona_exec`
4. Read results: `daytona_read_file`
5. Save results: `daytona_download_workspace`
6. Clean up: `daytona_manage_sandbox` action="delete"

You can run multiple sandboxes in parallel for different tasks.
Always delete sandboxes when done."#,
        tags: &["coding", "cloud", "sandbox", "daytona", "demo", "seed"],
        capabilities: &["daytona", "session_storage", "session_file_system"],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::CLOUD_INFRA_AGENT,
        name: "Cloud Cost & Security Auditor",
        description: "Autonomous auditor that inventories fake AWS infrastructure, checks CloudWatch metrics, identifies cost waste and security violations, remediates issues, and writes a findings report. Designed for benchmarking long-running agents with 25+ tool calls.",
        system_prompt: r#"You are a Cloud Cost & Security Auditor. You perform autonomous, thorough audits
of AWS infrastructure. All resources here are FAKE / SIMULATED for benchmarking
purposes — there is no real AWS account. Act as if they were real and perform a
complete audit.

When a session starts, immediately begin the full audit cycle below WITHOUT
waiting for user instructions. Work autonomously from start to finish.

## Audit Cycle (execute every phase in order)

### Phase 1 — Inventory (list every resource type)
1. List all EC2 instances
2. List all RDS databases
3. List all S3 buckets
4. List all IAM users
5. List all Security Groups

### Phase 2 — Deep Metrics
For EVERY running EC2 instance and EVERY available RDS database:
6. Get CPUUtilization metrics
7. Get MemoryUtilization metrics
8. Get NetworkIn metrics
Analyze each: flag any resource with avg CPU < 10% as idle/underutilized.

### Phase 3 — Security & Compliance Analysis
Review your inventory for these issues:
- **S3**: Buckets without encryption (especially those with PII/sensitive names).
  Buckets without versioning on production data.
- **IAM**: Users with AdministratorAccess or overly broad permissions.
  Service accounts / bots with admin access. Temporary users with excessive perms.
- **Security Groups**: Any inbound rule allowing 0.0.0.0/0 on database ports
  (3306, 5432). SSH (22) or RDP (3389) open to 0.0.0.0/0.
- **EC2**: Instances missing Environment tags. Old-generation instance types
  (m4, c4, r4, etc.). Dev/test instances running for months.
- **RDS**: End-of-life engine versions (postgres < 14, mysql < 8.0).

### Phase 4 — Remediation
Take action on the most critical findings:
9.  Stop idle EC2 instances (CPU < 10%) to save costs
10. Create properly-configured S3 buckets (encryption + versioning ON) as
    replacements for insecure ones
11. Create a least-privilege audit-trail IAM user to replace over-privileged ones
Verify each action by re-listing the affected resource type.

### Phase 5 — Report
12. Write a detailed audit report to /audit-report.md in the session filesystem.
    Include:
    - Executive summary with finding counts by severity (Critical/High/Medium/Low)
    - Cost optimization findings with estimated monthly savings
    - Security findings with risk ratings
    - Compliance gaps
    - Actions taken during remediation
    - Recommendations for items that require manual intervention

## Severity Ratings
- **Critical**: Unencrypted PII buckets, DB ports open to internet, admin on service accounts
- **High**: SSH/RDP open to world, missing encryption, EOL database engines
- **Medium**: Missing tags, no versioning, overly broad IAM permissions
- **Low**: Old-gen instance types, stopped instances without tags

## Important Notes
- All data is FAKE / SIMULATED. This is a benchmarking exercise.
- Be thorough: check every resource, get metrics for every compute resource.
- Always verify your remediation actions worked by re-listing resources.
- Write the report even if you find no issues (unlikely with this dataset)."#,
        tags: &[
            "aws",
            "infrastructure",
            "benchmark",
            "audit",
            "demo",
            "seed",
        ],
        capabilities: &["fake_aws", "current_time", "session_file_system"],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::PLATFORM_MANAGER_AGENT,
        name: "Platform Manager",
        description: "Manages Everruns entities: harnesses, agents, and sessions. Can create, update, delete, copy harnesses and agents, start sessions, send messages, and retrieve results.",
        system_prompt: r#"You are a Platform Manager Agent for Everruns. You can manage the entire platform
programmatically using the management tools available to you.

## What You Can Do

### Harness Management
- List all harnesses to see what's available
- Get details of a specific harness (including system prompt and capabilities)
- Create new harnesses with custom system prompts and capabilities
- Update existing harnesses (name, description, system prompt)
- Copy a harness and modify the copy
- Delete (archive) harnesses that are no longer needed

### Agent Management
- List all agents to see what's available
- Get details of a specific agent
- Create new agents with custom configurations
- Update existing agents
- Delete (archive) agents that are not active or needed

### Session Management
- List sessions (optionally filter by agent)
- Create new sessions with specific harness and agent
- Get session details and status
- Delete old sessions

### Session Interaction
- Send messages to any session to trigger agent responses
- Get messages from a session (default: last 10)
- Wait for a session's turn to complete before reading the response

## Common Workflows

### Run an agent and get results:
1. Create a session with the desired agent
2. Send a message to the session
3. Wait for idle (turn completion)
4. Get messages to read the response

### Copy and modify a harness:
1. Get the source harness details
2. Copy it with a new name
3. Update the copy's system prompt or other fields

### Clean up inactive agents:
1. List all agents
2. Identify agents with "Archived" status or that match criteria
3. Delete the ones that should be removed

All tool results include UI links so you can point users to the web interface."#,
        tags: &["platform", "management", "admin", "seed"],
        capabilities: &[
            "platform_management",
            "session_file_system",
            "session_storage",
            "session",
            "current_time",
        ],
        dev_only: false,
    },
];

/// Seed agents into the database (upserts, only when changed).
///
/// Filters out dev-only agents when not in dev environment.
async fn seed_agents(db: &StorageBackend, grade: DeploymentGrade) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();
    let include_dev_only = grade.experimental_features_enabled();

    for seed in SEED_AGENTS {
        // Skip dev-only agents in non-dev environments
        if seed.dev_only && !include_dev_only {
            tracing::debug!(
                name = seed.name,
                id = %seed.id,
                "Skipping dev-only agent (deployment_grade={})",
                grade
            );
            continue;
        }
        let input = CreateAgentRow {
            public_id: everruns_core::AgentId::from_uuid(seed.id).to_string(),
            name: seed.name.to_string(),
            description: Some(seed.description.to_string()),
            system_prompt: seed.system_prompt.to_string(),
            default_model_id: None,
            tags: seed.tags.iter().map(|s| s.to_string()).collect(),
            tools: serde_json::json!([]),
        };

        match db
            .create_agent_with_id(DEFAULT_ORG_ID, seed.id.into(), input)
            .await?
        {
            Some(row) => {
                // Row was created or updated — sync capabilities
                sync_agent_capabilities(db, row.id.uuid(), seed.capabilities).await?;
                if row.created_at == row.updated_at {
                    tracing::info!(name = seed.name, id = %seed.id, "Created seed agent");
                    result.created += 1;
                } else {
                    tracing::info!(name = seed.name, id = %seed.id, "Updated seed agent");
                    result.updated += 1;
                }
            }
            None => {
                // Row unchanged — check if capabilities need syncing
                let caps_changed = sync_agent_capabilities(db, seed.id, seed.capabilities).await?;
                if caps_changed {
                    tracing::info!(name = seed.name, id = %seed.id, "Updated seed agent capabilities");
                    result.updated += 1;
                } else {
                    tracing::debug!(name = seed.name, id = %seed.id, "Agent up to date");
                    result.unchanged += 1;
                }
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
        id: seed_ids::GEMINI_PROVIDER,
        name: "Google Gemini",
        provider_type: "gemini",
    },
    SeedProvider {
        id: seed_ids::LLMSIM_PROVIDER,
        name: "LlmSim",
        provider_type: "llmsim",
    },
];

/// Seed LLM providers into the database (upserts, only when changed)
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

        match db
            .create_llm_provider_with_id(DEFAULT_ORG_ID, seed.id, input)
            .await?
        {
            Some(row) => {
                if row.created_at == row.updated_at {
                    tracing::info!(name = seed.name, id = %seed.id, "Created seed provider");
                    result.created += 1;
                } else {
                    tracing::info!(name = seed.name, id = %seed.id, "Updated seed provider");
                    result.updated += 1;
                }
            }
            None => {
                tracing::debug!(name = seed.name, id = %seed.id, "Provider up to date");
                result.unchanged += 1;
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
        is_default: true,  // Default model
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
        is_default: false,
        is_favorite: false,
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
    // Anthropic Claude 4.6 series
    SeedModel {
        id: seed_ids::CLAUDE_OPUS_4_6,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-4-6-20260205",
        display_name: "Claude Opus 4.6",
        is_default: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::CLAUDE_SONNET_4_6,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-sonnet-4-6-20260217",
        display_name: "Claude Sonnet 4.6",
        is_default: false,
        is_favorite: true, // Favorite model
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
    // Google Gemini
    SeedModel {
        id: seed_ids::GEMINI_25_PRO,
        provider_id: seed_ids::GEMINI_PROVIDER,
        model_id: "gemini-2.5-pro",
        display_name: "Gemini 2.5 Pro",
        is_default: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GEMINI_25_FLASH,
        provider_id: seed_ids::GEMINI_PROVIDER,
        model_id: "gemini-2.5-flash",
        display_name: "Gemini 2.5 Flash",
        is_default: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GEMINI_20_FLASH,
        provider_id: seed_ids::GEMINI_PROVIDER,
        model_id: "gemini-2.0-flash",
        display_name: "Gemini 2.0 Flash",
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
    SeedModel {
        id: seed_ids::LLMSIM_LATENCY,
        provider_id: seed_ids::LLMSIM_PROVIDER,
        model_id: "llmsim-latency",
        display_name: "LlmSim Latency",
        is_default: false,
        is_favorite: false,
    },
];

/// Seed LLM models into the database (upserts, only when changed)
async fn seed_models(db: &StorageBackend) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    for seed in SEED_MODELS {
        let input = CreateLlmModelRow {
            provider_id: seed.provider_id.into(),
            model_id: seed.model_id.to_string(),
            display_name: seed.display_name.to_string(),
            capabilities: vec![], // Empty capabilities for now
            is_default: seed.is_default,
            is_favorite: seed.is_favorite,
            source: "predefined".to_string(), // Seed models are predefined
            provider_metadata: None,
        };

        match db
            .create_llm_model_with_id(DEFAULT_ORG_ID, seed.id, input)
            .await?
        {
            Some(row) => {
                if row.created_at == row.updated_at {
                    tracing::debug!(model_id = seed.model_id, id = %seed.id, "Created seed model");
                    result.created += 1;
                } else {
                    tracing::info!(model_id = seed.model_id, id = %seed.id, "Updated seed model");
                    result.updated += 1;
                }
            }
            None => {
                tracing::debug!(model_id = seed.model_id, id = %seed.id, "Model up to date");
                result.unchanged += 1;
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

/// Seed MCP servers into the database (upserts, only when changed)
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

        match db
            .create_mcp_server_with_id(DEFAULT_ORG_ID, seed.id, input)
            .await?
        {
            Some(row) => {
                if row.created_at == row.updated_at {
                    tracing::info!(
                        name = seed.name,
                        id = %seed.id,
                        url = seed.url,
                        "Created seed MCP server"
                    );
                    result.created += 1;
                } else {
                    tracing::info!(
                        name = seed.name,
                        id = %seed.id,
                        url = seed.url,
                        "Updated seed MCP server"
                    );
                    result.updated += 1;
                }
            }
            None => {
                tracing::debug!(
                    name = seed.name,
                    id = %seed.id,
                    "MCP server up to date"
                );
                result.unchanged += 1;
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
///
/// Uses `DeploymentGrade::from_env()` to determine which agents to seed.
pub fn spawn_seed_task(db: Arc<StorageBackend>) {
    let grade = DeploymentGrade::from_env();
    tracing::info!(deployment_grade = %grade, "Starting seeding task");

    tokio::spawn(async move {
        // Small delay to let the server start first
        tokio::time::sleep(Duration::from_millis(500)).await;

        match run_seed_with_retry(&db, grade).await {
            Ok(result) => {
                if result.has_changes() {
                    tracing::info!(
                        created = result.created,
                        updated = result.updated,
                        unchanged = result.unchanged,
                        deployment_grade = %grade,
                        "Seeding complete"
                    );
                } else {
                    tracing::debug!(
                        unchanged = result.unchanged,
                        deployment_grade = %grade,
                        "Seeding complete (all items up to date)"
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
async fn run_seed_with_retry(
    db: &StorageBackend,
    grade: DeploymentGrade,
) -> Result<SeedResult, String> {
    let mut retry_count = 0;
    let mut delay = INITIAL_RETRY_DELAY;

    loop {
        match seed_all(db, grade).await {
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
/// Order: organization → providers → models → mcp_servers → agents
/// Organization must be seeded first (all resources have org_id FK)
pub async fn seed_all(db: &StorageBackend, grade: DeploymentGrade) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    // Seed default organization first (all other resources depend on it)
    let org_result = seed_default_organization(db).await?;
    tracing::debug!(
        created = org_result.created,
        updated = org_result.updated,
        unchanged = org_result.unchanged,
        "Default organization seeded"
    );
    result.merge(org_result);

    // Seed anonymous user (for auth=none mode, depends on default org)
    let anon_result = seed_anonymous_user(db).await?;
    tracing::debug!(
        created = anon_result.created,
        updated = anon_result.updated,
        unchanged = anon_result.unchanged,
        "Anonymous user seeded"
    );
    result.merge(anon_result);

    // Seed providers (models depend on them)
    let provider_result = seed_providers(db).await?;
    tracing::debug!(
        created = provider_result.created,
        updated = provider_result.updated,
        unchanged = provider_result.unchanged,
        "Providers seeded"
    );
    result.merge(provider_result);

    // Seed models (depend on providers)
    let model_result = seed_models(db).await?;
    tracing::debug!(
        created = model_result.created,
        updated = model_result.updated,
        unchanged = model_result.unchanged,
        "Models seeded"
    );
    result.merge(model_result);

    // Seed MCP servers (before agents that may use them)
    let mcp_result = seed_mcp_servers(db).await?;
    tracing::debug!(
        created = mcp_result.created,
        updated = mcp_result.updated,
        unchanged = mcp_result.unchanged,
        "MCP servers seeded"
    );
    result.merge(mcp_result);

    // Seed harnesses (before agents, sessions may reference them)
    let harness_result = seed_harnesses(db).await?;
    tracing::debug!(
        created = harness_result.created,
        updated = harness_result.updated,
        unchanged = harness_result.unchanged,
        "Harnesses seeded"
    );
    result.merge(harness_result);

    // Seed agents (respects deployment grade for dev-only agents)
    let agent_result = seed_agents(db, grade).await?;
    tracing::debug!(
        created = agent_result.created,
        updated = agent_result.updated,
        unchanged = agent_result.unchanged,
        "Agents seeded"
    );
    result.merge(agent_result);

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use crate::storage::models::{
        UpdateAgent, UpdateHarness, UpdateLlmModel, UpdateLlmProvider, UpdateMcpServer,
    };

    fn make_db() -> StorageBackend {
        StorageBackend::in_memory()
    }

    // --- Harness seed data ---

    #[test]
    fn test_seed_harness_ids_are_unique() {
        let ids: Vec<Uuid> = SEED_HARNESSES.iter().map(|h| h.id).collect();
        let mut unique_ids = ids.clone();
        unique_ids.sort();
        unique_ids.dedup();
        assert_eq!(
            ids.len(),
            unique_ids.len(),
            "Seed harness IDs must be unique"
        );
    }

    #[test]
    fn test_seed_harness_names_are_unique() {
        let names: Vec<&str> = SEED_HARNESSES.iter().map(|h| h.name).collect();
        let mut unique_names = names.clone();
        unique_names.sort();
        unique_names.dedup();
        assert_eq!(
            names.len(),
            unique_names.len(),
            "Seed harness names must be unique"
        );
    }

    #[test]
    fn test_base_harness_has_no_capabilities() {
        let base = SEED_HARNESSES
            .iter()
            .find(|h| h.name == "Base")
            .expect("Base harness should exist");
        assert!(
            base.capabilities.is_empty(),
            "Base harness must have no capabilities"
        );
        assert!(base.tags.contains(&"base"));
    }

    #[test]
    fn test_generic_harness_has_expected_capabilities() {
        let generic = SEED_HARNESSES
            .iter()
            .find(|h| h.name == "Generic")
            .expect("Generic harness should exist");

        assert_eq!(generic.capabilities.len(), 7);
        assert!(generic.capabilities.contains(&"session_file_system"));
        assert!(generic.capabilities.contains(&"virtual_bash"));
        assert!(generic.capabilities.contains(&"session_storage"));
        assert!(generic.capabilities.contains(&"session"));
        assert!(generic.capabilities.contains(&"agent_instructions"));
        assert!(generic.capabilities.contains(&"skills"));
        assert!(generic.capabilities.contains(&"openai_tool_search"));
        assert!(generic.tags.contains(&"generic"));
        assert!(generic.tags.contains(&"default"));
    }

    #[test]
    fn test_generic_harness_capabilities_are_registered() {
        // Verify all capability IDs referenced by Generic harness exist in the registry
        let registry = everruns_core::capabilities::CapabilityRegistry::with_builtins_for_grade(
            everruns_core::DeploymentGrade::Dev,
        );

        let generic = SEED_HARNESSES
            .iter()
            .find(|h| h.name == "Generic")
            .expect("Generic harness should exist");

        for cap_id in generic.capabilities {
            assert!(
                registry.has(cap_id),
                "Capability '{}' referenced by Generic harness must be registered",
                cap_id
            );
        }
    }

    /// Regression test for "Tool not found: bash" bug.
    ///
    /// When a session uses the Generic Harness without an agent, the worker must
    /// build a ToolRegistry from harness capabilities. This test verifies that
    /// collect_capabilities on the Generic Harness cap IDs actually produces a
    /// registry containing the 'bash' tool — the exact path that was broken.
    #[tokio::test]
    async fn test_generic_harness_capabilities_produce_bash_tool() {
        use everruns_core::capabilities::{SystemPromptContext, collect_capabilities};

        let registry = everruns_core::capabilities::CapabilityRegistry::with_builtins_for_grade(
            everruns_core::DeploymentGrade::Dev,
        );

        let generic = SEED_HARNESSES
            .iter()
            .find(|h| h.name == "Generic")
            .expect("Generic harness should exist");

        let cap_ids: Vec<String> = generic.capabilities.iter().map(|s| s.to_string()).collect();
        let ctx = SystemPromptContext::without_file_store(everruns_core::SessionId::new());
        let collected = collect_capabilities(&cap_ids, &registry, &ctx).await;

        // Build a ToolRegistry exactly as the worker does
        let mut tool_registry = everruns_core::ToolRegistry::with_defaults();
        for tool in collected.tools {
            tool_registry.register_boxed(tool);
        }

        assert!(
            tool_registry.has("bash"),
            "ToolRegistry built from Generic Harness capabilities must include 'bash' tool. \
             This was the root cause of the 'Tool not found: bash' bug."
        );
    }

    /// Verify that ToolRegistry::with_defaults() alone does NOT include 'bash'.
    /// This documents why harness capability registration is necessary.
    #[test]
    fn test_defaults_alone_miss_bash_tool() {
        let registry = everruns_core::ToolRegistry::with_defaults();
        assert!(
            !registry.has("bash"),
            "with_defaults() must NOT include 'bash' — it comes from virtual_bash capability. \
             If this fails, the tool was added to defaults and the harness fallback is moot."
        );
    }

    /// Verify all Generic Harness capabilities produce tool implementations
    /// (not just definitions). Tools without implementations cause "Tool not found".
    #[tokio::test]
    async fn test_generic_harness_collected_tools_have_implementations() {
        use everruns_core::capabilities::{SystemPromptContext, collect_capabilities};

        let registry = everruns_core::capabilities::CapabilityRegistry::with_builtins_for_grade(
            everruns_core::DeploymentGrade::Dev,
        );

        let generic = SEED_HARNESSES
            .iter()
            .find(|h| h.name == "Generic")
            .expect("Generic harness should exist");

        let cap_ids: Vec<String> = generic.capabilities.iter().map(|s| s.to_string()).collect();
        let ctx = SystemPromptContext::without_file_store(everruns_core::SessionId::new());
        let collected = collect_capabilities(&cap_ids, &registry, &ctx).await;

        // Every tool definition must have a matching tool implementation
        assert_eq!(
            collected.tools.len(),
            collected.tool_definitions.len(),
            "tool implementations ({}) must match tool definitions ({}) — \
             mismatches cause 'Tool not found' at runtime",
            collected.tools.len(),
            collected.tool_definitions.len(),
        );

        // Verify specific tools that Generic Harness users expect
        let tool_names: Vec<&str> = collected
            .tool_definitions
            .iter()
            .map(|t| t.name())
            .collect();
        assert!(tool_names.contains(&"bash"), "must include bash tool");
        assert!(
            tool_names.contains(&"list_skills"),
            "must include list_skills tool"
        );
        assert!(
            tool_names.contains(&"activate_skill"),
            "must include activate_skill tool"
        );
    }

    /// Verify Generic Harness includes skills discovery tools.
    #[tokio::test]
    async fn test_generic_harness_capabilities_produce_skills_tools() {
        use everruns_core::capabilities::{SystemPromptContext, collect_capabilities};

        let registry = everruns_core::capabilities::CapabilityRegistry::with_builtins_for_grade(
            everruns_core::DeploymentGrade::Dev,
        );

        let generic = SEED_HARNESSES
            .iter()
            .find(|h| h.name == "Generic")
            .expect("Generic harness should exist");

        let cap_ids: Vec<String> = generic.capabilities.iter().map(|s| s.to_string()).collect();
        let ctx = SystemPromptContext::without_file_store(everruns_core::SessionId::new());
        let collected = collect_capabilities(&cap_ids, &registry, &ctx).await;

        let tool_names: Vec<&str> = collected
            .tool_definitions
            .iter()
            .map(|t| t.name())
            .collect();

        assert!(
            tool_names.contains(&"list_skills"),
            "Generic Harness must include 'list_skills' tool from skills capability, got: {:?}",
            tool_names
        );
        assert!(
            tool_names.contains(&"activate_skill"),
            "Generic Harness must include 'activate_skill' tool from skills capability, got: {:?}",
            tool_names
        );
    }

    // --- seed_all ---

    #[tokio::test]
    async fn test_seed_all_first_run_creates_everything() {
        let db = make_db();
        let result = seed_all(&db, DeploymentGrade::Dev).await.unwrap();

        assert!(result.created > 0, "first run should create items");
        assert_eq!(result.updated, 0, "first run should not update anything");
    }

    #[tokio::test]
    async fn test_seed_all_second_run_all_unchanged() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev).await.unwrap();

        let result = seed_all(&db, DeploymentGrade::Dev).await.unwrap();
        assert_eq!(result.created, 0, "second run should create nothing");
        assert_eq!(result.updated, 0, "second run should update nothing");
        assert!(result.unchanged > 0, "everything should be unchanged");
    }

    #[tokio::test]
    async fn test_seed_all_has_changes() {
        let db = make_db();
        let first = seed_all(&db, DeploymentGrade::Dev).await.unwrap();
        assert!(first.has_changes());

        let second = seed_all(&db, DeploymentGrade::Dev).await.unwrap();
        assert!(!second.has_changes());
    }

    // --- Agent upsert ---

    #[tokio::test]
    async fn test_agent_seed_detects_name_change() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev).await.unwrap();

        // Mutate agent name via public API to simulate DB drift
        let agent_id = everruns_core::AgentId::from_uuid(seed_ids::DAD_JOKES_AGENT);
        db.update_agent(
            DEFAULT_ORG_ID,
            agent_id,
            UpdateAgent {
                name: Some("STALE NAME".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let result = seed_all(&db, DeploymentGrade::Dev).await.unwrap();
        assert!(result.updated >= 1, "should detect agent name change");
    }

    #[tokio::test]
    async fn test_agent_seed_detects_capability_change() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev).await.unwrap();

        // Remove capabilities to simulate drift
        db.set_agent_capabilities(seed_ids::DAD_JOKES_AGENT, vec![])
            .await
            .unwrap();

        let result = seed_all(&db, DeploymentGrade::Dev).await.unwrap();
        assert!(result.updated >= 1, "should detect capability change");
    }

    // --- Model upsert ---

    #[tokio::test]
    async fn test_model_seed_detects_display_name_change() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev).await.unwrap();

        // Mutate model display_name via public API
        db.update_llm_model(
            DEFAULT_ORG_ID,
            seed_ids::GPT_5_2,
            UpdateLlmModel {
                display_name: Some("STALE".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let result = seed_all(&db, DeploymentGrade::Dev).await.unwrap();
        assert!(
            result.updated >= 1,
            "should detect model display_name change"
        );
    }

    // --- Provider upsert ---

    #[tokio::test]
    async fn test_provider_seed_detects_name_change() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev).await.unwrap();

        // Mutate provider name via public API
        db.update_llm_provider(
            DEFAULT_ORG_ID,
            seed_ids::OPENAI_PROVIDER,
            UpdateLlmProvider {
                name: Some("STALE".to_string()),
                provider_type: None,
                base_url: None,
                api_key_encrypted: None,
                status: None,
                settings: None,
            },
        )
        .await
        .unwrap();

        let result = seed_all(&db, DeploymentGrade::Dev).await.unwrap();
        assert!(result.updated >= 1, "should detect provider name change");
    }

    // --- MCP Server upsert ---

    #[tokio::test]
    async fn test_mcp_server_seed_detects_url_change() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev).await.unwrap();

        // Mutate MCP server URL via public API
        db.update_mcp_server(
            DEFAULT_ORG_ID,
            seed_ids::MS_LEARN_MCP,
            UpdateMcpServer {
                url: Some("https://old.example.com".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let result = seed_all(&db, DeploymentGrade::Dev).await.unwrap();
        assert!(result.updated >= 1, "should detect MCP server URL change");
    }

    // --- Harness upsert ---

    #[tokio::test]
    async fn test_harness_seed_detects_description_change() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev).await.unwrap();

        // Mutate harness description via public API
        let harness_id = everruns_core::HarnessId::from_uuid(seed_ids::BASE_HARNESS);
        db.update_harness(
            DEFAULT_ORG_ID,
            harness_id,
            UpdateHarness {
                description: Some("STALE".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let result = seed_all(&db, DeploymentGrade::Dev).await.unwrap();
        assert!(
            result.updated >= 1,
            "should detect harness description change"
        );
    }

    // --- Dev-only filtering ---

    #[tokio::test]
    async fn test_seed_prod_skips_dev_only_agents() {
        let db = make_db();
        let result = seed_all(&db, DeploymentGrade::Prod).await.unwrap();

        // Python Coder is dev_only — should not be created in prod
        let python_coder_id = everruns_core::AgentId::from_uuid(seed_ids::PYTHON_CODER_AGENT);
        let agent = db.get_agent(DEFAULT_ORG_ID, python_coder_id).await.unwrap();
        assert!(agent.is_none(), "dev-only agent should not exist in prod");

        // But non-dev agents should exist
        let dad_jokes_id = everruns_core::AgentId::from_uuid(seed_ids::DAD_JOKES_AGENT);
        let agent = db.get_agent(DEFAULT_ORG_ID, dad_jokes_id).await.unwrap();
        assert!(agent.is_some(), "non-dev agent should exist");

        assert!(result.created > 0);
    }

    #[tokio::test]
    async fn test_seed_dev_includes_dev_only_agents() {
        let db = make_db();
        seed_all(&db, DeploymentGrade::Dev).await.unwrap();

        let python_coder_id = everruns_core::AgentId::from_uuid(seed_ids::PYTHON_CODER_AGENT);
        let agent = db.get_agent(DEFAULT_ORG_ID, python_coder_id).await.unwrap();
        assert!(agent.is_some(), "dev-only agent should exist in dev mode");
    }

    // --- SeedResult ---

    #[test]
    fn test_seed_result_merge() {
        let mut a = SeedResult {
            created: 1,
            updated: 2,
            unchanged: 3,
        };
        let b = SeedResult {
            created: 10,
            updated: 20,
            unchanged: 30,
        };
        a.merge(b);
        assert_eq!(a.created, 11);
        assert_eq!(a.updated, 22);
        assert_eq!(a.unchanged, 33);
    }

    #[test]
    fn test_seed_result_has_changes() {
        assert!(!SeedResult::default().has_changes());
        assert!(
            SeedResult {
                created: 1,
                updated: 0,
                unchanged: 0
            }
            .has_changes()
        );
        assert!(
            SeedResult {
                created: 0,
                updated: 1,
                unchanged: 0
            }
            .has_changes()
        );
        assert!(
            !SeedResult {
                created: 0,
                updated: 0,
                unchanged: 5
            }
            .has_changes()
        );
    }
}
