// Service seeding module for control-plane
// Decision: Embed seed data directly to avoid file dependencies
// Decision: Check by name before insert to prevent duplicates
// Decision: Retry on database connection failure

use anyhow::Result;
use everruns_control_plane::storage::{models::CreateAgentRow, StorageBackend};
use std::sync::Arc;
use std::time::Duration;

/// Seed agent definition
struct SeedAgent {
    name: &'static str,
    description: &'static str,
    system_prompt: &'static str,
    tags: &'static [&'static str],
    capabilities: &'static [&'static str],
}

/// Built-in seed agents
const SEED_AGENTS: &[SeedAgent] = &[
    SeedAgent {
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

/// Maximum number of retries when database is not ready
const MAX_RETRIES: u32 = 10;

/// Initial retry delay (increases exponentially)
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(1);

/// Run seeding with retry logic for database readiness
pub async fn run_seed(db: Arc<StorageBackend>) -> Result<()> {
    tracing::info!("Starting service seeding...");

    let mut retry_count = 0;
    let mut delay = INITIAL_RETRY_DELAY;

    loop {
        match seed_agents(&db).await {
            Ok((created, skipped)) => {
                tracing::info!(created = created, skipped = skipped, "Seeding complete");
                return Ok(());
            }
            Err(e) => {
                retry_count += 1;
                if retry_count > MAX_RETRIES {
                    tracing::error!("Seeding failed after {} retries: {}", MAX_RETRIES, e);
                    return Err(e);
                }

                tracing::warn!(
                    attempt = retry_count,
                    max_retries = MAX_RETRIES,
                    delay_secs = delay.as_secs(),
                    error = %e,
                    "Database not ready, retrying..."
                );

                tokio::time::sleep(delay).await;
                // Exponential backoff, max 30 seconds
                delay = std::cmp::min(delay * 2, Duration::from_secs(30));
            }
        }
    }
}

/// Seed agents into the database, skipping existing ones
async fn seed_agents(db: &StorageBackend) -> Result<(usize, usize)> {
    let mut created = 0;
    let mut skipped = 0;

    for seed in SEED_AGENTS {
        // Check if agent with this name already exists
        match db.get_agent_by_name(seed.name).await? {
            Some(_) => {
                tracing::debug!(name = seed.name, "Agent already exists, skipping");
                skipped += 1;
            }
            None => {
                // Create the agent
                let input = CreateAgentRow {
                    name: seed.name.to_string(),
                    description: Some(seed.description.to_string()),
                    system_prompt: seed.system_prompt.to_string(),
                    default_model_id: None,
                    tags: seed.tags.iter().map(|s| s.to_string()).collect(),
                };

                let row = db.create_agent(input).await?;
                let agent_id = row.id;

                // Set capabilities if any
                if !seed.capabilities.is_empty() {
                    let cap_tuples: Vec<(String, i32)> = seed
                        .capabilities
                        .iter()
                        .enumerate()
                        .map(|(idx, cap)| (cap.to_string(), idx as i32))
                        .collect();
                    db.set_agent_capabilities(agent_id, cap_tuples).await?;
                }

                tracing::info!(name = seed.name, id = %agent_id, "Created seed agent");
                created += 1;
            }
        }
    }

    Ok((created, skipped))
}
