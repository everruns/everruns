use async_trait::async_trait;
use chrono::Utc;
use everruns_core::capabilities::{Capability, CapabilityStatus};
use everruns_core::driver_registry::DriverRegistry;
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::session_file::InitialFile;
use everruns_core::tool_types::{ToolCall, ToolHints};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{
    Agent, AgentCapabilityConfig, AgentStatus, CapabilityRegistry, DriverId, Harness,
    HarnessStatus, Message, PlatformDefinition, ResolvedModel, Session, SessionStatus,
};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

type ExampleError = Box<dyn std::error::Error + Send + Sync>;
type ExampleResult<T> = Result<T, ExampleError>;

struct WeekendConciergeCapability;

impl Capability for WeekendConciergeCapability {
    fn id(&self) -> &str {
        "weekend_concierge"
    }

    fn name(&self) -> &str {
        "Weekend Concierge"
    }

    fn description(&self) -> &str {
        "Host-owned local venue suggestions for small group outings."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("sparkles")
    }

    fn category(&self) -> Option<&str> {
        Some("Examples")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "Use `lookup_neighborhood_spot` when you need real venue facts from the host app. \
             Check `/workspace/weekend-brief.md` for the group's constraints before you answer.",
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(LookupNeighborhoodSpotTool)]
    }
}

#[derive(Clone, Serialize)]
struct Spot {
    name: &'static str,
    neighborhood: &'static str,
    vibe: &'static str,
    best_for: &'static [&'static str],
    max_group_size: u64,
    budget_per_person_usd: u64,
    signature: &'static str,
}

fn curated_spots() -> Vec<Spot> {
    vec![
        Spot {
            name: "Neon Arcade",
            neighborhood: "West Loop",
            vibe: "playful",
            best_for: &["arcade", "team outing", "snacks"],
            max_group_size: 10,
            budget_per_person_usd: 34,
            signature: "Retro cabinets, pinball, and surprisingly good grilled cheese.",
        },
        Spot {
            name: "Moonrise Mini Golf",
            neighborhood: "River North",
            vibe: "bright",
            best_for: &["mini golf", "celebration", "group"],
            max_group_size: 8,
            budget_per_person_usd: 38,
            signature: "Indoor mini golf with late-night desserts and mocktails.",
        },
        Spot {
            name: "Quiet Boardroom Cafe",
            neighborhood: "Logan Square",
            vibe: "cozy",
            best_for: &["board games", "conversation", "coffee"],
            max_group_size: 6,
            budget_per_person_usd: 22,
            signature: "Hundreds of board games, comfy booths, and zero pressure to rush.",
        },
    ]
}

struct LookupNeighborhoodSpotTool;

#[async_trait]
impl Tool for LookupNeighborhoodSpotTool {
    fn name(&self) -> &str {
        "lookup_neighborhood_spot"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Lookup Neighborhood Spot")
    }

    fn description(&self) -> &str {
        "Return curated local outing ideas from the host application's private venue list."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "occasion": {
                    "type": "string",
                    "description": "What the group is trying to plan, for example team outing or date night."
                },
                "group_size": {
                    "type": "integer",
                    "description": "How many people need to fit."
                },
                "vibe": {
                    "type": "string",
                    "description": "Preferred energy, for example playful, cozy, or bright."
                }
            },
            "required": ["occasion", "group_size", "vibe"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let occasion = match arguments.get("occasion").and_then(Value::as_str) {
            Some(occasion) => occasion.to_lowercase(),
            None => {
                return ToolExecutionResult::tool_error(
                    "Invalid arguments: 'occasion' is required and must be a string.",
                );
            }
        };
        let group_size = match arguments.get("group_size").and_then(Value::as_u64) {
            Some(group_size) => group_size,
            None => {
                return ToolExecutionResult::tool_error(
                    "Invalid arguments: 'group_size' is required and must be an unsigned integer.",
                );
            }
        };
        let vibe = match arguments.get("vibe").and_then(Value::as_str) {
            Some(vibe) => vibe.to_lowercase(),
            None => {
                return ToolExecutionResult::tool_error(
                    "Invalid arguments: 'vibe' is required and must be a string.",
                );
            }
        };

        let mut matches: Vec<_> = curated_spots()
            .into_iter()
            .filter(|spot| group_size <= spot.max_group_size)
            .filter(|spot| {
                vibe == "any"
                    || spot.vibe.eq_ignore_ascii_case(&vibe)
                    || spot
                        .best_for
                        .iter()
                        .any(|tag| tag.eq_ignore_ascii_case(&vibe))
            })
            .filter(|spot| {
                occasion == "outing"
                    || spot
                        .best_for
                        .iter()
                        .any(|tag| tag.eq_ignore_ascii_case(&occasion))
            })
            .collect();

        if matches.is_empty() {
            matches = curated_spots()
                .into_iter()
                .filter(|spot| group_size <= spot.max_group_size)
                .collect();
        }

        ToolExecutionResult::success(json!({
            "occasion": occasion,
            "group_size": group_size,
            "vibe": vibe,
            "recommendations": matches,
            "host_note": "This tool is backed by the host app's own curated list, not a remote API."
        }))
    }
}

fn harness(harness_id: everruns_core::HarnessId) -> Harness {
    Harness {
        id: harness_id,
        name: "weekend-concierge".into(),
        display_name: Some("Weekend Concierge".into()),
        description: Some("A hosted in-process concierge for planning fun local outings.".into()),
        system_prompt: Some(
            "You are a concise local concierge. Use host tools when venue facts matter.".into(),
        ),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec!["example".into(), "embedded".into()],
        capabilities: vec![AgentCapabilityConfig::new("weekend_concierge")],
        mcp_servers: Default::default(),
        initial_files: vec![InitialFile {
            path: "/workspace/welcome-note.md".into(),
            content: "This session is hosted entirely in-process by the embedding app.".into(),
            encoding: "text".into(),
            is_readonly: true,
        }],
        network_access: None,
        parallel_tool_calls: None,
        embedder_metadata: Default::default(),
        is_built_in: false,
        status: HarnessStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        archived_at: None,
        deleted_at: None,
    }
}

fn agent(
    agent_id: everruns_core::AgentId,
    harness_id: everruns_core::HarnessId,
    default_model: Option<everruns_core::typed_id::ModelId>,
) -> Agent {
    Agent {
        public_id: agent_id,
        internal_id: Uuid::nil(),
        name: "weekend-concierge-agent".into(),
        display_name: Some("Weekend Concierge Agent".into()),
        description: Some("Plans low-friction local hangs for small groups.".into()),
        system_prompt:
            "Keep the recommendation practical: one primary pick, one backup, one reason each."
                .into(),
        default_model_id: default_model,
        harness_id,
        default_version_id: None,
        forked_from_agent_id: None,
        forked_from_version_id: None,
        root_agent_id: None,
        tags: vec!["example".into()],
        capabilities: vec![],
        mcp_servers: Default::default(),
        initial_files: vec![],
        network_access: None,
        max_iterations: Some(6),
        parallel_tool_calls: None,
        tools: vec![],
        status: AgentStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        archived_at: None,
        deleted_at: None,
        usage: None,
    }
}

fn session(
    session_id: everruns_core::typed_id::SessionId,
    harness_id: everruns_core::HarnessId,
    agent_id: everruns_core::AgentId,
) -> Session {
    Session {
        source: Default::default(),
        activity: Default::default(),
        id: session_id,
        workspace_id: everruns_core::WorkspaceId::from_uuid((session_id).uuid()),
        organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
        harness_id,
        agent_id: Some(agent_id),
        agent_version_id: None,
        agent_identity_id: None,
        owner_principal_id: everruns_core::PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        owner: None,
        effective_owner: None,
        title: Some("Friday Team Outing".into()),
        goal: None,
        locale: Some("en-US".into()),
        preview: None,
        output_preview: None,
        tags: vec!["example".into()],
        model_id: None,
        capabilities: vec![],
        mcp_servers: Default::default(),
        tools: vec![],
        system_prompt: None,
        initial_files: vec![],
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        status: SessionStatus::Started,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_at: None,
        finished_at: None,
        usage: None,
        is_pinned: None,
        active_schedule_count: None,
        features: vec![],
        parent_session_id: None,
        forked_from_session_id: None,
        forked_from_sequence: None,
        blueprint_id: None,
        blueprint_config: None,
    }
}

fn message_preview(message: &Message) -> String {
    if let Some(text) = message.text() {
        return text.to_string();
    }
    if let Some(tool_call_id) = message.tool_call_id() {
        return format!("<tool result for {tool_call_id}>");
    }
    "<non-text content>".to_string()
}

pub struct ExampleRun {
    pub tool_names: Vec<String>,
    pub seeded_brief: String,
    pub final_response: String,
    pub success: bool,
    pub iterations: usize,
    pub tool_calls_count: usize,
    pub transcript: Vec<String>,
    pub event_types: Vec<String>,
}

pub async fn run_weekend_concierge_demo() -> ExampleResult<ExampleRun> {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(WeekendConciergeCapability);
    let platform = PlatformDefinition::new(capabilities, DriverRegistry::new());

    let harness_id = everruns_core::HarnessId::new();
    let agent_id = everruns_core::AgentId::new();
    let session_id = everruns_core::typed_id::SessionId::new();

    let runtime = everruns_runtime::InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .llm_sim(
            LlmSimConfig::sequence(vec![
                "Let me check the concierge book before I recommend anything.".into(),
                "Go with Neon Arcade in the West Loop. It fits six people, stays under the budget, and gives the group an easy mix of games and snacks. Backup: Quiet Boardroom Cafe if the team wants something calmer.".into(),
            ])
            .with_tool_call_sequence(vec![
                vec![ToolCall {
                    id: "call_spot_1".into(),
                    name: "lookup_neighborhood_spot".into(),
                    arguments: json!({
                        "occasion": "team outing",
                        "group_size": 6,
                        "vibe": "playful"
                    }),
                }],
                vec![],
            ])
            .with_model("llmsim-weekend-concierge"),
        )
        .default_model(ResolvedModel {
            model: "llmsim-weekend-concierge".into(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
        })
        .harness(harness(harness_id))
        .agent(agent(agent_id, harness_id, None))
        .session(session(session_id, harness_id, agent_id))
        .seed_text_file(
            session_id,
            "/workspace/weekend-brief.md",
            "Group: 6 coworkers\nBudget: under $40/person\nMood: playful but low-planning\nMust-have: snacks or desserts\nBackup option: quieter venue if people are tired",
        )
        .build()
        .await?;

    let context = runtime.load_context(session_id).await?;
    let tool_names = context
        .runtime_agent
        .tools
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();

    let brief = runtime
        .read_file(session_id, "/workspace/weekend-brief.md")
        .await?
        .expect("seeded brief file")
        .content
        .unwrap_or_default();

    let result = runtime
        .run_text_turn(
            session_id,
            "Find a Friday outing for six teammates. Keep it playful, snack-friendly, and under $40 per person.",
        )
        .await?;

    let messages = runtime.messages(session_id).await?;
    let transcript = messages
        .iter()
        .map(|message| format!("[{}] {}", message.role, message_preview(message)))
        .collect();

    let event_types = runtime
        .events()
        .await?
        .into_iter()
        .map(|event| event.event_type)
        .collect();

    Ok(ExampleRun {
        tool_names,
        seeded_brief: brief,
        final_response: result.response,
        success: result.success,
        iterations: result.iterations,
        tool_calls_count: result.tool_calls_count,
        transcript,
        event_types,
    })
}

#[cfg(test)]
mod tests {
    use super::{LookupNeighborhoodSpotTool, run_weekend_concierge_demo};
    use everruns_core::tools::{Tool, ToolExecutionResult};
    use serde_json::json;

    #[tokio::test]
    async fn weekend_concierge_demo_stays_deterministic() {
        let run = run_weekend_concierge_demo()
            .await
            .expect("example run should succeed");

        assert_eq!(run.tool_names, vec!["lookup_neighborhood_spot"]);
        assert!(run.seeded_brief.contains("Budget: under $40/person"));
        assert!(run.final_response.contains("Neon Arcade"));
        assert!(run.final_response.contains("Quiet Boardroom Cafe"));
        assert!(run.success);
        assert_eq!(run.iterations, 2);
        assert_eq!(run.tool_calls_count, 1);
        assert!(
            run.transcript
                .iter()
                .any(|line| line.contains("<tool result for call_spot_1>"))
        );
        assert!(
            run.event_types
                .iter()
                .any(|event| event == "tool.completed")
        );
        assert!(
            run.event_types
                .iter()
                .any(|event| event == "reason.completed")
        );
    }

    #[tokio::test]
    async fn lookup_tool_rejects_missing_required_arguments() {
        let result = LookupNeighborhoodSpotTool
            .execute(json!({
                "occasion": "team outing",
                "group_size": 6
            }))
            .await;

        match result {
            ToolExecutionResult::ToolError(message) => {
                assert_eq!(
                    message,
                    "Invalid arguments: 'vibe' is required and must be a string."
                );
            }
            other => panic!("expected tool error, got {other:?}"),
        }
    }
}
