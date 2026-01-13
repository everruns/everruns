// Service seeding module for control-plane
// Decision: Always seed on startup (no flag needed)
// Decision: Run in background task (non-blocking)
// Decision: Failures are non-fatal (log warning, continue)
// Decision: Use fixed UUIDs for idempotency (ON CONFLICT DO NOTHING)

use everruns_control_plane::storage::{models::CreateAgentRow, StorageBackend};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Well-known UUIDs for seed agents (following the pattern from providers)
/// Format: 01933b5a-0000-7000-8000-0000000001xx
mod seed_ids {
    use uuid::Uuid;

    pub const DAD_JOKES_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000101);
    pub const RESEARCH_AGENT: Uuid = Uuid::from_u128(0x01933b5a_0000_7000_8000_000000000102);
}

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
];

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
            Ok((created, skipped)) => {
                if created > 0 {
                    tracing::info!(created = created, skipped = skipped, "Seeding complete");
                } else {
                    tracing::debug!(skipped = skipped, "Seeding complete (all agents exist)");
                }
            }
            Err(e) => {
                // Non-fatal: log warning and continue
                // This can happen if:
                // - Migrations haven't been applied yet
                // - Database is temporarily unavailable
                // - Schema doesn't exist
                tracing::warn!(
                    error = %e,
                    "Seeding failed (non-fatal). Seed agents may not be available."
                );
            }
        }
    });
}

/// Run seeding with retry logic for transient errors
async fn run_seed_with_retry(db: &StorageBackend) -> Result<(usize, usize), String> {
    let mut retry_count = 0;
    let mut delay = INITIAL_RETRY_DELAY;

    loop {
        match seed_agents(db).await {
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

/// Seed agents into the database using fixed UUIDs (idempotent)
/// Uses ON CONFLICT DO NOTHING for atomic idempotency
async fn seed_agents(db: &StorageBackend) -> anyhow::Result<(usize, usize)> {
    let mut created = 0;
    let mut skipped = 0;

    for seed in SEED_AGENTS {
        let input = CreateAgentRow {
            name: seed.name.to_string(),
            description: Some(seed.description.to_string()),
            system_prompt: seed.system_prompt.to_string(),
            default_model_id: None,
            tags: seed.tags.iter().map(|s| s.to_string()).collect(),
        };

        // Use create_agent_with_id which does ON CONFLICT DO NOTHING
        // Returns None if agent already exists
        match db.create_agent_with_id(seed.id, input).await? {
            Some(row) => {
                // Set capabilities if any
                if !seed.capabilities.is_empty() {
                    let cap_tuples: Vec<(String, i32)> = seed
                        .capabilities
                        .iter()
                        .enumerate()
                        .map(|(idx, cap)| (cap.to_string(), idx as i32))
                        .collect();
                    db.set_agent_capabilities(row.id, cap_tuples).await?;
                }

                tracing::info!(name = seed.name, id = %seed.id, "Created seed agent");
                created += 1;
            }
            None => {
                tracing::debug!(name = seed.name, id = %seed.id, "Agent already exists, skipping");
                skipped += 1;
            }
        }
    }

    Ok((created, skipped))
}
