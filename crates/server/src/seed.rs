// Service seeding module for server (control plane)
// Decision: Always seed on startup (no flag needed)
// Decision: Run in background task (non-blocking)
// Decision: Failures are non-fatal (log warning, continue)
// Decision: Use fixed UUIDs for idempotency
// Decision: Upsert seed data on conflict, only when values changed (ON CONFLICT DO UPDATE ... WHERE differs)
// Decision: Modular design allows easy addition of new seeders

use crate::auth::config::{AdminConfig, AuthMode};
use crate::org_init;
use crate::storage::{
    StorageBackend,
    models::{
        CreateLlmModelRow, CreateLlmProviderRow, CreateMcpServerRow, CreateOrganizationRow,
        CreateUserRow,
    },
    password::hash_password,
};
use everruns_core::{
    ANONYMOUS_USER_EMAIL, ANONYMOUS_USER_ID, ANONYMOUS_USER_NAME, DEFAULT_ORG_ID,
    DEFAULT_ORG_PUBLIC_ID, DeploymentGrade, PlatformDefinition,
};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Well-known UUID for the Generic harness (default for sessions without explicit harness)
pub const GENERIC_HARNESS_ID: Uuid = org_init::GENERIC_HARNESS_ID;

/// Well-known UUID for the Chat harness (used by global chat endpoint)
pub const CHAT_HARNESS_ID: Uuid = org_init::CHAT_HARNESS_ID;

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
    pub const DAYTONA_CODER_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000108);
    pub const E2B_CODER_AGENT: Uuid = Uuid::from_u128(0x0195bb5a_0000_7000_8000_00000000010f);
    pub const DENO_CODER_AGENT: Uuid = Uuid::from_u128(0x0195bb5a_0000_7000_8000_000000000110);
    pub const SPRITES_CODER_AGENT: Uuid = Uuid::from_u128(0x0195bb5a_0000_7000_8000_000000000111);
    pub const CLOUD_INFRA_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000109);
    pub const PLATFORM_MANAGER_AGENT: Uuid =
        Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000010a);
    pub const WEB_RESEARCHER_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000010b);
    pub const BROWSER_TESTER_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000010c);
    pub const DASHBOARD_BUILDER_AGENT: Uuid =
        Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000010d);
    pub const TASK_ORCHESTRATOR_AGENT: Uuid =
        Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000010e);
    pub const KNOWLEDGE_BASE_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000112);

    // MCP Servers (0x500-0x5FF)
    pub const MS_LEARN_MCP: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000501);

    // Harnesses (0x600-0x6FF) — now managed by org_init module

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
    pub const GPT_5_4: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000021f);
    pub const GPT_5_4_PRO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000220);
    pub const GPT_5_4_MINI: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000221);
    pub const GPT_5_4_NANO: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000222);
    pub const O1_PREVIEW: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000021e);

    // Anthropic Models (0x300-0x3FF)
    pub const CLAUDE_OPUS_4_6: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000309);
    pub const CLAUDE_SONNET_4_6: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000030a);
    pub const CLAUDE_HAIKU_4_6: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_00000000030b);
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

    // Ensure default org has built-in harnesses (same safety net as registration handlers)
    org_init::initialize_org_harnesses(db, DEFAULT_ORG_ID).await?;

    Ok(result)
}

// ============================================
// Admin User Seeder
// ============================================

/// Seed admin user for auth=admin mode.
/// Creates the admin user at startup so they have an org membership
/// before first login. Uses a well-known UUID for idempotency.
const ADMIN_USER_ID: Uuid = Uuid::from_u128(0x00000000_0000_0000_0000_000000000002);

async fn seed_admin_user(
    db: &StorageBackend,
    admin_config: &AdminConfig,
) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    let password_hash = hash_password(&admin_config.password)
        .map_err(|e| anyhow::anyhow!("Failed to hash admin password: {}", e))?;

    let input = CreateUserRow {
        email: admin_config.email.clone(),
        name: "Admin".to_string(),
        avatar_url: None,
        roles: vec!["admin".to_string()],
        password_hash: Some(password_hash),
        email_verified: true,
        auth_provider: Some("local".to_string()),
        auth_provider_id: None,
        external_id: None,
    };

    // Try creating with well-known ID; if user already exists by email,
    // look them up instead (admin may have been created via login before
    // this seeder existed).
    match db.create_user_with_id(ADMIN_USER_ID, input).await? {
        Some(_) => {
            tracing::info!("Created admin user");
            result.created += 1;
        }
        None => {
            tracing::debug!("Admin user up to date");
            result.unchanged += 1;
        }
    }

    // Resolve the actual user id — may differ from ADMIN_USER_ID if the
    // admin was created via login (which assigns a random UUID).
    let user_id = db
        .get_user_by_email(&admin_config.email)
        .await?
        .map(|u| u.id)
        .unwrap_or(ADMIN_USER_ID);

    // Ensure admin user is owner of default org
    db.ensure_membership(user_id, DEFAULT_ORG_ID, "owner")
        .await?;

    Ok(result)
}

// ============================================
// Agent Seeder
// ============================================

/// Capability entry with optional per-capability config.
pub(crate) struct SeedCapability {
    pub(crate) id: &'static str,
    pub(crate) config: Option<fn() -> serde_json::Value>,
}

impl SeedCapability {
    pub(crate) const fn new(id: &'static str) -> Self {
        Self { id, config: None }
    }

    pub(crate) const fn with_config(id: &'static str, config: fn() -> serde_json::Value) -> Self {
        Self {
            id,
            config: Some(config),
        }
    }
}

impl std::fmt::Display for SeedCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id)
    }
}
/// Seed agent definition (used by agent examples API, not auto-seeded into DB)
pub(crate) struct SeedAgent {
    /// Retained for stable identification; not auto-seeded into DB.
    #[allow(dead_code)]
    pub(crate) id: Uuid,
    /// Unique name (e.g. "dad-jokes-agent").
    pub(crate) name: &'static str,
    /// Human-readable display name (e.g. "Dad Jokes Agent").
    pub(crate) display_name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) system_prompt: &'static str,
    pub(crate) tags: &'static [&'static str],
    pub(crate) capabilities: &'static [SeedCapability],
    /// If true, only seed in dev environments (experimental features)
    pub(crate) dev_only: bool,
}

/// Built-in seed agents
pub(crate) const SEED_AGENTS: &[SeedAgent] = &[
    SeedAgent {
        id: seed_ids::DAD_JOKES_AGENT,
        name: "dad-jokes-agent",
        display_name: "Dad Jokes Agent",
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
        capabilities: &[SeedCapability::new("current_time")],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::RESEARCH_AGENT,
        name: "research-agent",
        display_name: "Research Agent",
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
        capabilities: &[
            SeedCapability::new("stateless_todo_list"),
            SeedCapability::with_config(
                "web_fetch",
                || serde_json::json!({"enable_file_download": true}),
            ),
            SeedCapability::new("session_file_system"),
        ],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::MS_LEARN_AGENT,
        name: "microsoft-learn-assistant",
        display_name: "Microsoft Learn Assistant",
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
        capabilities: &[SeedCapability::new(
            "mcp:01933b5a-0000-7000-8000-000000000501",
        )],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::PYTHON_CODER_AGENT,
        name: "python-coder",
        display_name: "Python Coder",
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
        capabilities: &[SeedCapability::new("docker_container")],
        dev_only: true, // Experimental capability, only in dev environments
    },
    SeedAgent {
        id: seed_ids::SHELL_ASSISTANT_AGENT,
        name: "shell-assistant",
        display_name: "Shell Assistant",
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
        capabilities: &[
            SeedCapability::new("virtual_bash"),
            SeedCapability::new("session_file_system"),
        ],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::DATA_ANALYST_AGENT,
        name: "data-analyst",
        display_name: "Data Analyst",
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
            SeedCapability::new("session_sql_database"),
            SeedCapability::new("session_file_system"),
            SeedCapability::new("stateless_todo_list"),
        ],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::DAYTONA_CODER_AGENT,
        name: "daytona-coder",
        display_name: "Daytona Coder",
        description: "A coding agent that runs code in cloud sandboxes powered by Daytona",
        system_prompt: r#"You are a Daytona Coder Agent. You run code in cloud sandboxes powered by Daytona.

Just call sandbox tools directly — the API key is resolved automatically from Settings > Connections or session secrets.

Workflow:
1. Create sandbox: `daytona_create_sandbox` (working directory: /home/daytona)
2. Clone repos: `daytona_git_clone` (clones to /home/daytona/owner/repo)
3. Write code / install deps: `daytona_write_file`, `daytona_exec`
4. Read results: `daytona_read_file`
5. Save results: `daytona_download_workspace`
6. Clean up: `daytona_manage_sandbox` action="delete"

You can run multiple sandboxes in parallel for different tasks.
Always delete sandboxes when done."#,
        tags: &["coding", "cloud", "sandbox", "daytona", "demo", "seed"],
        capabilities: &[
            SeedCapability::new("daytona"),
            SeedCapability::new("session_storage"),
            SeedCapability::new("session_file_system"),
        ],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::E2B_CODER_AGENT,
        name: "e2b-coder",
        display_name: "E2B Coder",
        description: "A coding agent that runs code in cloud sandboxes powered by E2B",
        system_prompt: r#"You are an E2B Coder Agent. You run code in cloud sandboxes powered by E2B.

The E2B API key is resolved automatically from the platform environment or session secrets.

Workflow:
1. Create sandbox: `e2b_create_sandbox` (workspace: /home/user)
2. Write files / install deps: `e2b_write_file`, `e2b_exec`
3. Read results: `e2b_read_file`
4. Inspect active sandboxes: `e2b_list_sandboxes`
5. Clean up: `e2b_manage_sandbox` action="pause" or action="delete"

You can run multiple sandboxes in parallel for separate tasks.
Delete sandboxes when work is complete; pause only when the user explicitly wants to keep state around."#,
        tags: &["coding", "cloud", "sandbox", "e2b", "demo", "seed"],
        capabilities: &[
            SeedCapability::new("e2b"),
            SeedCapability::new("session_storage"),
            SeedCapability::new("session_file_system"),
        ],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::DENO_CODER_AGENT,
        name: "deno-coder",
        display_name: "Deno Coder",
        description: "A coding agent that runs code in cloud sandboxes powered by Deno",
        system_prompt: r#"You are a Deno Coder Agent. You run code in cloud sandboxes powered by Deno.

Just call sandbox tools directly — the access token is resolved automatically from Settings > Connections or environment variables.

Workflow:
1. Create sandbox: `deno_create_sandbox` (working directory: /home/sandbox)
2. Write code / install deps: `deno_write_file`, `deno_exec`
3. Read results: `deno_read_file`
4. Inspect active sandboxes: `deno_list_sandboxes`
5. Clean up: `deno_manage_sandbox` action="delete"

You can run multiple sandboxes in parallel for different tasks.
Always delete sandboxes when done."#,
        tags: &["coding", "cloud", "sandbox", "deno", "demo", "seed"],
        capabilities: &[
            SeedCapability::new("deno"),
            SeedCapability::new("session_storage"),
            SeedCapability::new("session_file_system"),
        ],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::SPRITES_CODER_AGENT,
        name: "sprites-coder",
        display_name: "Sprites Coder",
        description: "A coding agent that runs code in persistent Firecracker microVMs powered by Sprites",
        system_prompt: r#"You are a Sprites Coder Agent. You run code in persistent, hardware-isolated Linux microVMs powered by Sprites.

Just call tools directly — the API token is resolved automatically from Settings > Connections.

Workflow:
1. Create sprite: `sprites_create_sprite` (working directory: /home/user)
2. Write code / install deps: `sprites_write_file`, `sprites_exec`
3. Read results: `sprites_read_file`
4. Checkpoint before risky operations: `sprites_checkpoint`
5. Roll back if needed: `sprites_restore_checkpoint`
6. Expose HTTP services: listen on port 8080, get URL with `sprites_service_url`
7. Clean up: `sprites_manage_sprite` action="delete"

Key features:
- Persistent filesystem survives idle/hibernation
- Checkpoints snapshot state in ~300ms for safe rollback
- Each sprite has a public HTTP URL for serving web apps
- Instant wake from hibernation (<1s)

Always delete sprites when done to avoid storage charges."#,
        tags: &[
            "coding",
            "cloud",
            "sandbox",
            "sprites",
            "persistent",
            "demo",
            "seed",
        ],
        capabilities: &[
            SeedCapability::new("sprites"),
            SeedCapability::new("session_storage"),
            SeedCapability::new("session_file_system"),
        ],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::CLOUD_INFRA_AGENT,
        name: "cloud-cost-security-auditor",
        display_name: "Cloud Cost & Security Auditor",
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
        capabilities: &[
            SeedCapability::new("fake_aws"),
            SeedCapability::new("current_time"),
            SeedCapability::new("session_file_system"),
        ],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::PLATFORM_MANAGER_AGENT,
        name: "platform-manager",
        display_name: "Platform Manager",
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
            SeedCapability::new("platform_management"),
            SeedCapability::new("session_file_system"),
            SeedCapability::new("session_storage"),
            SeedCapability::new("session"),
            SeedCapability::new("current_time"),
        ],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::WEB_RESEARCHER_AGENT,
        name: "web-researcher",
        display_name: "Web Researcher",
        description: "An agent that searches the web using Brave Search to find current information, news, and documentation. Always cites sources with links.",
        system_prompt: r#"You are a Web Researcher Agent. You search the web using Brave Search to find
current, accurate information for any topic.

## How You Work

1. **Search the web** using `brave_web_search` to find relevant results
2. **Synthesize** findings into clear, well-organized responses
3. **Always cite sources** — include the URL for every piece of information

## Guidelines

- Use multiple searches to cross-reference information when needed
- Use the `freshness` parameter for time-sensitive queries (e.g., "pd" for past day, "pw" for past week)
- Always include source URLs so the user can verify information
- Format citations as markdown links: [Title](url)
- If search results are insufficient, say so honestly rather than speculating
- For broad topics, break them into specific sub-queries for better results

## Response Format

Structure your responses with:
- A concise summary at the top
- Detailed findings organized by subtopic
- A **Sources** section at the bottom listing all referenced URLs

## Example Sources Section

**Sources:**
- [Article Title](https://example.com/article)
- [Another Source](https://example.com/source)

## Prerequisites

Brave Search API key must be configured in Settings > Connections.
Get a free key at https://brave.com/search/api/"#,
        tags: &["research", "search", "web", "demo", "seed"],
        capabilities: &[
            SeedCapability::new("brave_search"),
            SeedCapability::new("stateless_todo_list"),
            SeedCapability::new("current_time"),
        ],
        dev_only: true,
    },
    SeedAgent {
        id: seed_ids::BROWSER_TESTER_AGENT,
        name: "browser-tester",
        display_name: "Browser Tester",
        description: "An agent that automates browser testing: navigates web pages, takes screenshots, reads DOM content, scrapes data, and interacts with UI elements (click, type, keyboard, mouse, touch). Useful for accessibility testing, regression testing, and web automation.",
        system_prompt: r#"You are a Browser Tester Agent. You automate browser interactions using Browserless.

## How You Work

1. **Navigate** to a URL with `browserless_navigate` to explore its structure (links, headings, meta)
2. **Screenshot** the page with `browserless_screenshot` for visual validation
3. **Read DOM** with `browserless_content` to inspect rendered HTML
4. **Scrape data** with `browserless_scrape` to extract structured information via CSS selectors
5. **Interact** with `browserless_interact` for multi-step flows (login, form filling, menu navigation)

## Interaction Capabilities

The `browserless_interact` tool supports:
- `click` — Click by CSS selector or x,y coordinates
- `type` — Type text into input fields
- `keyboard` — Press keys (Enter, Tab, Escape, etc.)
- `mouse_move` — Move mouse to coordinates
- `touch` — Tap elements (mobile simulation)
- `scroll` — Scroll the page
- `wait` / `wait_for_selector` — Wait for content to load
- `navigate` — Go to a different URL mid-interaction

Set `return_screenshot: true` to get a screenshot after interactions.

## Use Cases

- **Accessibility testing**: Navigate pages, read DOM, check ARIA attributes and heading structure
- **Regression testing**: Screenshot pages, compare with expected state, verify content
- **Login flows**: Use `browserless_interact` to fill login forms and verify post-login state
- **Content verification**: Scrape specific elements and validate their text/attributes
- **Visual QA**: Take screenshots before/after interactions to verify UI changes

## Secure Login Flows

For login-protected pages, use **secret references** to avoid exposing credentials:

1. Store credentials first: `secret_store set login_email user@example.com` and `secret_store set login_password s3cret`
2. In browserless_interact steps, set the value field to a secret reference pattern: dollar-sign followed by double-braces around secrets.NAME — for example a type step with value set to the pattern referencing secrets.login_password
3. Secrets are resolved server-side — you never see the plaintext values
4. Secret references only work in step value fields (not in url or navigate actions)

**Important**: Never ask users to paste credentials into chat. Always use secret_store + secret references in browserless_interact.

## Guidelines

- Each tool call uses a fresh browser — no state carries between calls
- Use `wait_for_selector` or `wait_for_timeout` for pages with dynamic content
- For login-protected pages, use secret references (see above) with `browserless_interact`
- Use `browserless_open_browser` for multi-step workflows that need persistent cookies/sessions
- Always clean up persistent sessions with `browserless_close_browser` when done

## Prerequisites

Browserless API token must be configured in Settings > Connections.
Get a token at https://www.browserless.io/account/home"#,
        tags: &[
            "browser",
            "testing",
            "automation",
            "a11y",
            "regression",
            "demo",
            "seed",
        ],
        capabilities: &[
            SeedCapability::new("browserless"),
            SeedCapability::new("session_storage"),
        ],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::DASHBOARD_BUILDER_AGENT,
        name: "dashboard-builder",
        display_name: "Dashboard Builder",
        description: "An agent that creates rich interactive dashboards, charts, tables, and forms using OpenUI components",
        system_prompt: r#"You are a Dashboard Builder Agent. You create rich interactive UI components —
charts, tables, forms, cards, and layouts — using OpenUI Lang.

## How You Work

1. **Generate immediately**: When the user asks for a dashboard, chart, table, or any visual — produce OpenUI code right away using realistic sample data. Do NOT ask clarifying questions first.
2. **Design the layout**: Combine multiple components for rich, complete dashboards.
3. **Iterate**: Refine the design based on user feedback.

## What You Can Build

- **Dashboards**: KPI cards, metric grids, status panels with multiple sections
- **Charts**: Bar, line, area, pie, radar, scatter plots with realistic data
- **Tables**: Data tables with column headers and rows
- **Forms**: Input fields, selects, checkboxes, radio buttons, sliders
- **Layouts**: Cards, grids, sections, tabs, accordions, steps, carousels

## Guidelines

- ALWAYS respond with OpenUI code in ```openui fenced code blocks — this is your primary output format
- ALWAYS start with `root = ...` so the UI shell renders immediately (leverages hoisting)
- Use realistic, plausible sample data that matches the user's domain
- Combine multiple components for rich dashboards (KPIs + charts + tables)
- Keep a brief explanation before or after the code block
- For dashboards, use Stack with direction "row" to create grid-like layouts

## Example

User: "Show me a sales dashboard"

Here's a sales dashboard with KPIs, revenue trend, and recent orders:

```openui
root = Stack([kpis, chart_section, orders_section])
kpis = Stack([kpi1, kpi2, kpi3], "row")
kpi1 = Card([CardHeader("Total Revenue", "$284,500")])
kpi2 = Card([CardHeader("Orders", "1,247")])
kpi3 = Card([CardHeader("Avg Order", "$228")])
chart_section = Card([chart_header, revenue_chart])
chart_header = CardHeader("Monthly Revenue", "Last 6 months")
revenue_chart = BarChart(months, [revenue_series])
months = ["Oct", "Nov", "Dec", "Jan", "Feb", "Mar"]
revenue_series = Series("Revenue", [38000, 42000, 51000, 45000, 52000, 56500])
orders_section = Card([orders_header, orders_table])
orders_header = CardHeader("Recent Orders", "Last 5 orders")
orders_table = Table(order_rows, [col_id, col_customer, col_amount, col_status])
col_id = Col("Order ID")
col_customer = Col("Customer")
col_amount = Col("Amount")
col_status = Col("Status")
order_rows = [["ORD-1247", "Acme Corp", "$3,200", "Shipped"], ["ORD-1246", "TechStart", "$1,850", "Processing"], ["ORD-1245", "GlobalFin", "$5,400", "Delivered"], ["ORD-1244", "DataFlow", "$2,100", "Shipped"], ["ORD-1243", "CloudNet", "$4,750", "Delivered"]]
```

This shows your key metrics at the top, revenue trend in the middle, and recent order activity below."#,
        tags: &["dashboard", "ui", "visualization", "openui", "demo", "seed"],
        capabilities: &[SeedCapability::new("openui")],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::TASK_ORCHESTRATOR_AGENT,
        name: "task-orchestrator",
        display_name: "Task Orchestrator",
        description: "An agent that breaks complex tasks into subtasks and delegates them to subagents for parallel execution. Coordinates results and synthesizes a final answer.",
        system_prompt: r#"You are a Task Orchestrator Agent. You break complex tasks into subtasks and delegate them to specialized subagents.

## How You Work

1. **Analyze the request**: Break down the user's task into independent subtasks
2. **Spawn subagents**: Create a named subagent for each subtask (e.g. "Research", "Code Review", "Test Runner")
3. **Monitor progress**: Use get_subagents to check on running subagents
4. **Steer if needed**: Use message_subagent to redirect or provide additional context
5. **Synthesize**: Combine subagent results into a coherent final response

## Guidelines

- Give subagents clear, specific task descriptions
- Use descriptive names ("Auth Analyzer" not "agent1")
- Spawn independent tasks in parallel when possible
- Review and synthesize subagent results before responding
- If a subagent fails, analyze the error and decide whether to retry or work around it

## Example Workflow

User: "Analyze my codebase and suggest improvements"
→ Spawn "Architecture Reviewer" to analyze overall structure
→ Spawn "Test Coverage Analyzer" to check test quality
→ Spawn "Dependency Auditor" to review dependencies
→ Wait for all to complete
→ Synthesize findings into a prioritized improvement plan"#,
        tags: &["orchestration", "subagents", "demo", "seed"],
        capabilities: &[
            SeedCapability::new("subagents"),
            SeedCapability::new("current_time"),
        ],
        dev_only: false,
    },
    SeedAgent {
        id: seed_ids::KNOWLEDGE_BASE_AGENT,
        name: "knowledge-base-agent",
        display_name: "Knowledge Base Agent",
        description: "An agent with persistent memory that learns from conversations, remembers facts, preferences, and corrections across sessions.",
        system_prompt: r#"You are a Knowledge Base Agent with persistent memory. You learn from every
conversation and remember important information across sessions.

## How You Work

1. **Remember**: When a user shares important facts, preferences, corrections, or procedures,
   save them with `remember`. Tag memories for easy retrieval.
2. **Recall**: Before answering questions, use `recall` to search your memory for relevant
   prior knowledge. This helps you give consistent, personalized responses.
3. **Forget**: If a user tells you something is outdated or wrong, use `forget` to remove
   the incorrect memory, then `remember` the corrected version.

## Guidelines

- Proactively recall relevant memories when the user asks a question
- Save user preferences (communication style, technical level, tools they use)
- Save corrections ("Actually, we use PostgreSQL not MySQL") immediately
- Tag memories descriptively for better recall (e.g. ["database", "preference"])
- Rate importance honestly: critical project facts = 8-10, nice-to-know = 3-5
- Don't save trivial or ephemeral information
- Tell the user when you're recalling something from a previous session"#,
        tags: &["memory", "knowledge", "learning", "seed"],
        capabilities: &[
            SeedCapability::new("memory"),
            SeedCapability::new("current_time"),
        ],
        dev_only: false,
    },
];

// Agents are NOT auto-seeded. They live as examples (agent_examples.rs) and are adopted on
// demand via POST /v1/agent-examples/{slug}/use. This prevents duplicate agents.

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

fn enabled_seed_provider_ids(platform_definition: &PlatformDefinition) -> HashSet<Uuid> {
    let registered_provider_types: HashSet<String> = platform_definition
        .driver_registry()
        .registered_providers()
        .into_iter()
        .map(|provider_type| provider_type.to_string())
        .collect();

    SEED_PROVIDERS
        .iter()
        .filter(|seed| registered_provider_types.contains(seed.provider_type))
        .map(|seed| seed.id)
        .collect()
}

async fn seed_providers_with_platform_definition(
    db: &StorageBackend,
    platform_definition: &PlatformDefinition,
) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();
    let enabled_provider_ids = enabled_seed_provider_ids(platform_definition);

    for seed in SEED_PROVIDERS {
        if !enabled_provider_ids.contains(&seed.id) {
            tracing::debug!(
                name = seed.name,
                id = %seed.id,
                provider_type = seed.provider_type,
                "Skipping seed provider because platform definition does not register its driver"
            );
            continue;
        }
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
    installed: bool,
    is_favorite: bool,
}

/// Built-in seed models
const SEED_MODELS: &[SeedModel] = &[
    // OpenAI GPT-5.4 series
    SeedModel {
        id: seed_ids::GPT_5_4,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.4",
        display_name: "GPT-5.4",
        installed: true,   // Installed by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_4_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.4-pro",
        display_name: "GPT-5.4 Pro",
        installed: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_4_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.4-mini",
        display_name: "GPT-5.4 mini",
        installed: true,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_4_NANO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.4-nano",
        display_name: "GPT-5.4 nano",
        installed: false,
        is_favorite: false,
    },
    // OpenAI GPT-5.2 series
    SeedModel {
        id: seed_ids::GPT_5_2,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.2",
        display_name: "GPT-5.2",
        installed: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_2_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.2-pro",
        display_name: "GPT-5.2 Pro",
        installed: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_2_CODEX,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.2-codex",
        display_name: "GPT-5.2 Codex",
        installed: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GPT_5_2_CHAT,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.2-chat-latest",
        display_name: "GPT-5.2 Chat",
        installed: false,
        is_favorite: false,
    },
    // OpenAI GPT-5.1 series
    SeedModel {
        id: seed_ids::GPT_5_1,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1",
        display_name: "GPT-5.1",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_1_CODEX,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1-codex",
        display_name: "GPT-5.1 Codex",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_1_CODEX_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1-codex-mini",
        display_name: "GPT-5.1 Codex mini",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_1_CODEX_MAX,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1-codex-max",
        display_name: "GPT-5.1 Codex max",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_1_CHAT,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5.1-chat-latest",
        display_name: "GPT-5.1 Chat",
        installed: false,
        is_favorite: false,
    },
    // OpenAI GPT-5 series
    SeedModel {
        id: seed_ids::GPT_5_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-mini",
        display_name: "GPT-5 mini",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5",
        display_name: "GPT-5",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_NANO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-nano",
        display_name: "GPT-5 nano",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-pro",
        display_name: "GPT-5 Pro",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_CODEX,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-codex",
        display_name: "GPT-5 Codex",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_5_CHAT,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-5-chat-latest",
        display_name: "GPT-5 Chat",
        installed: false,
        is_favorite: false,
    },
    // OpenAI GPT-4.1 series
    SeedModel {
        id: seed_ids::GPT_4_1,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-4.1",
        display_name: "GPT-4.1",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_4_1_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-4.1-mini",
        display_name: "GPT-4.1 mini",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_4_1_NANO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-4.1-nano",
        display_name: "GPT-4.1 nano",
        installed: false,
        is_favorite: false,
    },
    // OpenAI GPT-4 series
    SeedModel {
        id: seed_ids::GPT_4O,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-4o",
        display_name: "GPT-4o",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::GPT_4O_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "gpt-4o-mini",
        display_name: "GPT-4o mini",
        installed: false,
        is_favorite: false,
    },
    // OpenAI Reasoning models (o-series)
    SeedModel {
        id: seed_ids::O4_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o4-mini",
        display_name: "o4 mini",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O4_MINI_DEEP_RESEARCH,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o4-mini-deep-research",
        display_name: "o4 mini Deep Research",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O3,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o3",
        display_name: "o3",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O3_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o3-mini",
        display_name: "o3 mini",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O3_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o3-pro",
        display_name: "o3 Pro",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O3_DEEP_RESEARCH,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o3-deep-research",
        display_name: "o3 Deep Research",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O1,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o1",
        display_name: "o1",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O1_MINI,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o1-mini",
        display_name: "o1 mini",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O1_PRO,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o1-pro",
        display_name: "o1 Pro",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::O1_PREVIEW,
        provider_id: seed_ids::OPENAI_PROVIDER,
        model_id: "o1-preview",
        display_name: "o1 Preview",
        installed: false,
        is_favorite: false,
    },
    // Anthropic Claude 4.6 series
    SeedModel {
        id: seed_ids::CLAUDE_OPUS_4_6,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-4-6",
        display_name: "Claude Opus 4.6",
        installed: true,   // Installed by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::CLAUDE_SONNET_4_6,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-sonnet-4-6",
        display_name: "Claude Sonnet 4.6",
        installed: true,   // Installed by default
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::CLAUDE_HAIKU_4_6,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-haiku-4-6-20260301",
        display_name: "Claude Haiku 4.6",
        installed: true,   // Installed by default
        is_favorite: true, // Favorite model
    },
    // Anthropic Claude 4.5 series
    SeedModel {
        id: seed_ids::CLAUDE_OPUS_4_5,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-4-5",
        display_name: "Claude Opus 4.5",
        installed: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::CLAUDE_SONNET_4_5,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-sonnet-4-5",
        display_name: "Claude Sonnet 4.5",
        installed: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::CLAUDE_HAIKU_4_5,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-haiku-4-5",
        display_name: "Claude Haiku 4.5",
        installed: false,
        is_favorite: true, // Favorite model
    },
    // Anthropic Claude 4 series
    SeedModel {
        id: seed_ids::CLAUDE_OPUS_4,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-opus-4",
        display_name: "Claude Opus 4",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::CLAUDE_SONNET_4,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-sonnet-4",
        display_name: "Claude Sonnet 4",
        installed: false,
        is_favorite: false,
    },
    // Anthropic Claude 3.7
    SeedModel {
        id: seed_ids::CLAUDE_3_7_SONNET,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-3-7-sonnet-latest",
        display_name: "Claude 3.7 Sonnet",
        installed: false,
        is_favorite: false,
    },
    // Anthropic Claude 3.5
    SeedModel {
        id: seed_ids::CLAUDE_3_5_SONNET,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-3-5-sonnet-latest",
        display_name: "Claude 3.5 Sonnet",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::CLAUDE_3_5_HAIKU,
        provider_id: seed_ids::ANTHROPIC_PROVIDER,
        model_id: "claude-3-5-haiku-latest",
        display_name: "Claude 3.5 Haiku",
        installed: false,
        is_favorite: false,
    },
    // Google Gemini
    SeedModel {
        id: seed_ids::GEMINI_25_PRO,
        provider_id: seed_ids::GEMINI_PROVIDER,
        model_id: "gemini-2.5-pro",
        display_name: "Gemini 2.5 Pro",
        installed: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GEMINI_25_FLASH,
        provider_id: seed_ids::GEMINI_PROVIDER,
        model_id: "gemini-2.5-flash",
        display_name: "Gemini 2.5 Flash",
        installed: false,
        is_favorite: true, // Favorite model
    },
    SeedModel {
        id: seed_ids::GEMINI_20_FLASH,
        provider_id: seed_ids::GEMINI_PROVIDER,
        model_id: "gemini-2.0-flash",
        display_name: "Gemini 2.0 Flash",
        installed: false,
        is_favorite: false,
    },
    // LlmSim (simulated LLM for testing)
    SeedModel {
        id: seed_ids::LLMSIM_DEFAULT,
        provider_id: seed_ids::LLMSIM_PROVIDER,
        model_id: "llmsim-default",
        display_name: "LlmSim Default",
        installed: false,
        is_favorite: false,
    },
    SeedModel {
        id: seed_ids::LLMSIM_LATENCY,
        provider_id: seed_ids::LLMSIM_PROVIDER,
        model_id: "llmsim-latency",
        display_name: "LlmSim Latency",
        installed: false,
        is_favorite: false,
    },
];

/// Default model ID for org settings seeding (GPT-5.4)
const DEFAULT_MODEL_ID: Uuid = seed_ids::GPT_5_4;

/// Seed organization settings (default model, etc.)
async fn seed_organization_settings(db: &StorageBackend) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();

    match db.get_organization_settings(DEFAULT_ORG_ID).await? {
        Some(existing) if existing.default_model_id.is_some() => {
            tracing::debug!("Organization settings up to date");
            result.unchanged += 1;
        }
        _ => {
            db.upsert_organization_settings(DEFAULT_ORG_ID, Some(DEFAULT_MODEL_ID))
                .await?;
            tracing::info!("Seeded organization settings with default model");
            result.created += 1;
        }
    }

    Ok(result)
}

/// Seed LLM models into the database (upserts, only when changed)
async fn seed_models_with_platform_definition(
    db: &StorageBackend,
    platform_definition: &PlatformDefinition,
) -> anyhow::Result<SeedResult> {
    let mut result = SeedResult::default();
    let enabled_provider_ids = enabled_seed_provider_ids(platform_definition);

    for seed in SEED_MODELS {
        if !enabled_provider_ids.contains(&seed.provider_id) {
            tracing::debug!(
                model_id = seed.model_id,
                id = %seed.id,
                "Skipping seed model because its provider driver is not registered by the platform definition"
            );
            continue;
        }
        let input = CreateLlmModelRow {
            provider_id: seed.provider_id.into(),
            model_id: seed.model_id.to_string(),
            display_name: seed.display_name.to_string(),
            capabilities: vec![], // Empty capabilities for now
            installed: seed.installed,
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

/// Optional auth-mode context passed into the seed task so it can
/// pre-create the admin user when `AUTH_MODE=admin`.
#[derive(Clone, Default)]
pub struct SeedAuthContext {
    pub mode: AuthMode,
    pub admin: Option<AdminConfig>,
}

/// Spawn seeding as a background task (non-blocking)
/// This allows the HTTP server to start immediately while seeding runs in background.
/// Seeding failures are non-fatal - logged as warnings but don't crash the server.
///
/// Uses `DeploymentGrade::from_env()` to determine which agents to seed.
pub fn spawn_seed_task(db: Arc<StorageBackend>, auth_ctx: SeedAuthContext) {
    spawn_seed_task_with_platform_definition(
        db,
        auth_ctx,
        crate::platform::oss_platform_definition_for_grade(DeploymentGrade::from_env()),
    );
}

/// Spawn seeding as a background task using an explicit platform definition.
pub fn spawn_seed_task_with_platform_definition(
    db: Arc<StorageBackend>,
    auth_ctx: SeedAuthContext,
    platform_definition: PlatformDefinition,
) {
    let grade = DeploymentGrade::from_env();
    tracing::info!(deployment_grade = %grade, "Starting seeding task");

    tokio::spawn(async move {
        // Small delay to let the server start first
        tokio::time::sleep(Duration::from_millis(500)).await;

        match run_seed_with_retry(&db, grade, &auth_ctx, &platform_definition).await {
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

                // After seed succeeds, reconcile built-in harnesses for ALL orgs.
                // Runs inside the seed task (not a separate task) so the default
                // org row is guaranteed to exist before harness init runs.
                reconcile_org_harnesses(&db, &platform_definition).await;
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

/// Reconcile built-in harnesses for every organization (including the default org).
///
/// Called after seeding completes (inside the seed task) so the default org row
/// is guaranteed to exist. Each org is reconciled independently; a single failure
/// is logged but does not prevent other orgs from updating.
async fn reconcile_org_harnesses(db: &StorageBackend, platform_definition: &PlatformDefinition) {
    let harnesses = platform_definition.built_in_harnesses();
    let orgs = match db.list_organizations().await {
        Ok(orgs) => orgs,
        Err(e) => {
            tracing::warn!(error = %e, "Harness reconciliation: failed to list orgs (non-fatal)");
            return;
        }
    };

    if orgs.is_empty() {
        tracing::debug!("No orgs to reconcile harnesses for");
        return;
    }

    let mut created = 0usize;
    let mut updated = 0usize;
    let mut unchanged = 0usize;
    let mut errors = 0usize;

    for org in &orgs {
        match org_init::initialize_org_harnesses_with_definitions(db, org.org_id, harnesses).await {
            Ok(r) => {
                created += r.created;
                updated += r.updated;
                unchanged += r.unchanged;
            }
            Err(e) => {
                errors += 1;
                tracing::warn!(
                    org_id = org.org_id,
                    error = %e,
                    "Harness reconciliation failed for org (non-fatal)"
                );
            }
        }
    }

    if created > 0 || updated > 0 || errors > 0 {
        tracing::info!(
            org_count = orgs.len(),
            created,
            updated,
            unchanged,
            errors,
            "Background harness reconciliation complete"
        );
    } else {
        tracing::debug!(
            org_count = orgs.len(),
            unchanged,
            "Background harness reconciliation complete (all up to date)"
        );
    }
}

/// Run seeding with retry logic for transient errors
async fn run_seed_with_retry(
    db: &StorageBackend,
    grade: DeploymentGrade,
    auth_ctx: &SeedAuthContext,
    platform_definition: &PlatformDefinition,
) -> Result<SeedResult, String> {
    let mut retry_count = 0;
    let mut delay = INITIAL_RETRY_DELAY;

    loop {
        match seed_all_with_platform_definition(db, grade, auth_ctx, platform_definition).await {
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
/// Order: organization → users → providers → models → mcp_servers
/// Organization must be seeded first (all resources have org_id FK).
/// Harnesses are initialized by reconcile_org_harnesses (post-seed).
/// Note: Agents are NOT seeded; they live as examples and are adopted on demand.
pub async fn seed_all(
    db: &StorageBackend,
    grade: DeploymentGrade,
    auth_ctx: &SeedAuthContext,
) -> anyhow::Result<SeedResult> {
    let platform_definition = crate::platform::oss_platform_definition_for_grade(grade);
    seed_all_with_platform_definition(db, grade, auth_ctx, &platform_definition).await
}

/// Run all seeders in order using an explicit platform definition.
pub async fn seed_all_with_platform_definition(
    db: &StorageBackend,
    _grade: DeploymentGrade,
    auth_ctx: &SeedAuthContext,
    platform_definition: &PlatformDefinition,
) -> anyhow::Result<SeedResult> {
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

    // Seed admin user when in admin mode (depends on default org)
    if auth_ctx.mode == AuthMode::Admin
        && let Some(admin_config) = &auth_ctx.admin
    {
        let admin_result = seed_admin_user(db, admin_config).await?;
        tracing::debug!(
            created = admin_result.created,
            updated = admin_result.updated,
            unchanged = admin_result.unchanged,
            "Admin user seeded"
        );
        result.merge(admin_result);
    }

    // Seed providers (models depend on them)
    let provider_result = seed_providers_with_platform_definition(db, platform_definition).await?;
    tracing::debug!(
        created = provider_result.created,
        updated = provider_result.updated,
        unchanged = provider_result.unchanged,
        "Providers seeded"
    );
    result.merge(provider_result);

    // Seed models (depend on providers)
    let model_result = seed_models_with_platform_definition(db, platform_definition).await?;
    tracing::debug!(
        created = model_result.created,
        updated = model_result.updated,
        unchanged = model_result.unchanged,
        "Models seeded"
    );
    result.merge(model_result);

    // Seed organization settings (depends on models for default_model_id)
    let org_settings_result = seed_organization_settings(db).await?;
    tracing::debug!(
        created = org_settings_result.created,
        updated = org_settings_result.updated,
        unchanged = org_settings_result.unchanged,
        "Organization settings seeded"
    );
    result.merge(org_settings_result);

    // Seed MCP servers (before agents that may use them)
    let mcp_result = seed_mcp_servers(db).await?;
    tracing::debug!(
        created = mcp_result.created,
        updated = mcp_result.updated,
        unchanged = mcp_result.unchanged,
        "MCP servers seeded"
    );
    result.merge(mcp_result);

    // Built-in harnesses are initialized for ALL orgs (including the default org)
    // by reconcile_org_harnesses() which runs after seed_all completes.
    // Auth registration handlers also call initialize_org_harnesses as a safety
    // net, so harnesses are always present before a user interacts with an org.

    // Seed agents are available as examples (GET /v1/agent-examples) and adopted
    // on demand via POST /v1/agent-examples/{slug}/use. No automatic seeding —
    // this prevents duplicate agents when users adopt from the examples gallery.

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use crate::storage::models::{
        UpdateHarness, UpdateLlmModel, UpdateLlmProvider, UpdateMcpServer,
    };

    fn make_db() -> StorageBackend {
        StorageBackend::in_memory()
    }

    fn built_in_harnesses() -> Vec<everruns_core::BuiltInHarnessDefinition> {
        org_init::default_harness_definitions()
    }

    // --- Harness seed data ---

    #[test]
    fn test_seed_harness_ids_are_unique() {
        let ids: Vec<Uuid> = built_in_harnesses()
            .iter()
            .map(|h| {
                h.seed_id
                    .expect("OSS built-in harness should have a seed id")
            })
            .collect();
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
        let built_in_harnesses = built_in_harnesses();
        let names: Vec<&str> = built_in_harnesses.iter().map(|h| h.name.as_str()).collect();
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
        let built_in_harnesses = built_in_harnesses();
        let base = built_in_harnesses
            .iter()
            .find(|h| h.name == "base")
            .expect("Base harness should exist");
        assert!(
            base.capabilities.is_empty(),
            "Base harness must have no capabilities"
        );
        assert!(base.tags.iter().any(|tag| tag == "base"));
    }

    #[test]
    fn test_generic_harness_has_expected_capabilities() {
        let built_in_harnesses = built_in_harnesses();
        let generic = built_in_harnesses
            .iter()
            .find(|h| h.name == "generic")
            .expect("Generic harness should exist");

        let cap_ids: Vec<&str> = generic.capabilities.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(cap_ids.len(), 12);
        assert!(cap_ids.contains(&"session_file_system"));
        assert!(cap_ids.contains(&"virtual_bash"));
        assert!(cap_ids.contains(&"web_fetch"));
        assert!(cap_ids.contains(&"session_storage"));
        assert!(cap_ids.contains(&"session"));
        assert!(cap_ids.contains(&"agent_instructions"));
        assert!(cap_ids.contains(&"skills"));
        assert!(cap_ids.contains(&"infinity_context"));
        assert!(cap_ids.contains(&"openai_tool_search"));
        assert!(cap_ids.contains(&"budgeting"));
        assert!(cap_ids.contains(&"compaction"));
        assert!(cap_ids.contains(&"tool_output_persistence"));
        // Verify compaction default config
        let compaction_cap = generic
            .capabilities
            .iter()
            .find(|c| c.id == "compaction")
            .expect("compaction capability should exist");
        assert_eq!(compaction_cap.config["strategy"], "auto");
        assert_eq!(compaction_cap.config["proactive"], true);
        assert_eq!(compaction_cap.config["budget_percent"], 0.85);
        assert!(generic.tags.iter().any(|tag| tag == "generic"));
        assert!(generic.tags.iter().any(|tag| tag == "default"));
    }

    #[test]
    fn test_generic_harness_capabilities_are_registered() {
        // Verify all capability IDs referenced by Generic harness exist in the registry
        let registry = everruns_core::capabilities::CapabilityRegistry::with_builtins_for_grade(
            everruns_core::DeploymentGrade::Dev,
        );

        let built_in_harnesses = built_in_harnesses();
        let generic = built_in_harnesses
            .iter()
            .find(|h| h.name == "generic")
            .expect("Generic harness should exist");

        for cap in &generic.capabilities {
            assert!(
                registry.has(&cap.id),
                "Capability '{}' referenced by Generic harness must be registered",
                cap.id
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

        let built_in_harnesses = built_in_harnesses();
        let generic = built_in_harnesses
            .iter()
            .find(|h| h.name == "generic")
            .expect("Generic harness should exist");

        let cap_ids: Vec<String> = generic.capabilities.iter().map(|s| s.id.clone()).collect();
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

        let built_in_harnesses = built_in_harnesses();
        let generic = built_in_harnesses
            .iter()
            .find(|h| h.name == "generic")
            .expect("Generic harness should exist");

        let cap_ids: Vec<String> = generic.capabilities.iter().map(|s| s.id.clone()).collect();
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

        let built_in_harnesses = built_in_harnesses();
        let generic = built_in_harnesses
            .iter()
            .find(|h| h.name == "generic")
            .expect("Generic harness should exist");

        let cap_ids: Vec<String> = generic.capabilities.iter().map(|s| s.id.clone()).collect();
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
        let result = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

        assert!(result.created > 0, "first run should create items");
        assert_eq!(result.updated, 0, "first run should not update anything");

        // Harnesses are created by reconcile_org_harnesses (post-seed), not seed_all.
        // Simulate the full startup flow.
        let pd = crate::platform::oss_platform_definition_for_grade(DeploymentGrade::Dev);
        reconcile_org_harnesses(&db, &pd).await;

        let settings = db
            .get_organization_settings(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            settings.default_harness_id.unwrap().uuid(),
            org_init::GENERIC_HARNESS_ID
        );
        assert_eq!(
            settings.base_harness_id.unwrap().uuid(),
            org_init::BASE_HARNESS_ID
        );
    }

    #[tokio::test]
    async fn test_seed_all_second_run_all_unchanged() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

        let result = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        assert_eq!(result.created, 0, "second run should create nothing");
        assert_eq!(result.updated, 0, "second run should update nothing");
        assert!(result.unchanged > 0, "everything should be unchanged");
    }

    #[tokio::test]
    async fn test_seed_all_has_changes() {
        let db = make_db();
        let first = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        assert!(first.has_changes());

        let second = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        assert!(!second.has_changes());
    }

    // --- Agent seeding regression ---

    #[tokio::test]
    async fn test_seed_all_does_not_create_agents() {
        // Agents should NOT be auto-seeded; they live as examples and are adopted on demand.
        let db = make_db();
        seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

        let (agents, _) = db
            .list_agents(
                DEFAULT_ORG_ID,
                None,
                false,
                crate::api::common::Pagination::new(0, 100),
            )
            .await
            .unwrap();
        assert!(
            agents.is_empty(),
            "seed_all should not create agents; they are adopted from examples"
        );
    }

    // --- Model upsert ---

    #[tokio::test]
    async fn test_model_seed_detects_display_name_change() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

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

        let result = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        assert!(
            result.updated >= 1,
            "should detect model display_name change"
        );
    }

    // --- Provider upsert ---

    #[tokio::test]
    async fn test_provider_seed_detects_name_change() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

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

        let result = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        assert!(result.updated >= 1, "should detect provider name change");
    }

    // --- MCP Server upsert ---

    #[tokio::test]
    async fn test_mcp_server_seed_detects_url_change() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

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

        let result = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        assert!(result.updated >= 1, "should detect MCP server URL change");
    }

    // --- Harness upsert ---

    #[tokio::test]
    async fn test_harness_reconcile_detects_description_change() {
        let db = make_db();
        let _ = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();
        let pd = crate::platform::oss_platform_definition_for_grade(DeploymentGrade::Dev);
        reconcile_org_harnesses(&db, &pd).await;

        // Mutate harness description via public API
        let built_in_harnesses = built_in_harnesses();
        let harness_id = everruns_core::HarnessId::from_uuid(
            built_in_harnesses
                .iter()
                .find(|h| h.name == "base")
                .unwrap()
                .seed_id
                .expect("OSS built-in harness should have a seed id"),
        );
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

        // Reconciliation should detect the stale description and update it
        let result = org_init::initialize_org_harnesses(&db, DEFAULT_ORG_ID)
            .await
            .unwrap();
        assert!(
            result.updated >= 1,
            "should detect harness description change"
        );
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

    // --- Multi-org harness reconciliation ---

    #[tokio::test]
    async fn test_reconcile_org_harnesses() {
        use crate::storage::models::CreateOrganizationRow;

        let db = make_db();
        // Seed creates the org but no longer initialises harnesses inline.
        seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
            .await
            .unwrap();

        // Create a second org.
        let second_org = db
            .create_organization(CreateOrganizationRow {
                public_id: "org-second".into(),
                name: "Second Org".into(),
                created_by: None,
            })
            .await
            .unwrap();

        // Before reconciliation, second org should have no harnesses.
        let before = db
            .list_harnesses(second_org.org_id, None, false)
            .await
            .unwrap();
        assert!(
            before.is_empty(),
            "second org should start with no harnesses"
        );

        // Run reconciliation (covers ALL orgs, including default).
        let pd = crate::platform::oss_platform_definition_for_grade(DeploymentGrade::Dev);
        reconcile_org_harnesses(&db, &pd).await;

        // After reconciliation, both orgs should have the built-in harnesses.
        let default_harnesses = db
            .list_harnesses(DEFAULT_ORG_ID, None, false)
            .await
            .unwrap();
        assert!(
            !default_harnesses.is_empty(),
            "default org should have harnesses after reconciliation"
        );

        let second_harnesses = db
            .list_harnesses(second_org.org_id, None, false)
            .await
            .unwrap();
        assert_eq!(second_harnesses.len(), default_harnesses.len());
    }
}
