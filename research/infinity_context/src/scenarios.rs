//! Synthetic scenario generation
//!
//! Generates DatasetRecord with events for eval datasets.
//!
//! # Design: Token Budget
//!
//! The default eval context window is 50k tokens with 70% budget (35k tokens).
//! This ensures scenarios exceed the budget, triggering context trimming and
//! making the history search tool useful. Key considerations:
//!
//! - Filler messages are ~100-200 tokens each (detailed questions/answers)
//! - Tool outputs are ~1500-3000 tokens each (realistic large file reads)
//! - With ~300-500 events per scenario and many large tool results,
//!   total tokens significantly exceed 35k budget (target: 80k-150k tokens)
//! - The Infinity Context capability trims older messages and provides
//!   the query_history tool to search excluded content
//!
//! If scenarios don't exceed the budget, all messages stay in context and
//! query_history returns empty results (nothing was excluded).
//!
//! # Token Estimation
//!
//! Rough estimates (chars / 4 ≈ tokens):
//! - Large tool results: ~6000 chars = ~1500 tokens each
//! - With 40 tool results per scenario: ~60k tokens just from tools
//! - Plus filler messages: additional ~20-40k tokens
//! - Total per scenario: ~80-100k tokens (well over 35k budget)

use crate::dataset::{
    make_input_event, make_output_event, make_output_event_with_tool_call, make_tool_event,
    DatasetRecord, Event, Input, Meta, PlantedInfo, SessionId,
};

/// Generate synthetic test scenarios - one of each type
pub fn generate_synthetic() -> Vec<DatasetRecord> {
    vec![
        generate_needle_scenario(0),
        generate_multi_hop_scenario(0),
        generate_cumulative_scenario(0),
        generate_final_decision_scenario(0),
        generate_decision_timeline_scenario(0),
        generate_tool_disambiguation_scenario(0),
    ]
}

/// Generate a needle-in-haystack scenario
fn generate_needle_scenario(variant: usize) -> DatasetRecord {
    let session_id = SessionId::new();

    // Fake secrets for evaluation scenarios - NOT real credentials
    let secrets = [
        ("API_KEY", "fake-api-key-abc123xyz789"),
        ("DATABASE_PASSWORD", "fake-db-password-42"),
        ("JWT_SECRET", "fake-jwt-secret-key"),
        ("ACCESS_KEY", "FAKE-ACCESS-KEY-EXAMPLE"),
        ("PAYMENT_KEY", "fake-payment-key-example"),
    ];

    let (key_name, key_value) = secrets[variant % secrets.len()];
    // Plant needle early so it gets trimmed from context
    let needle_position = 25 + (variant * 7) % 40;
    // Much larger conversation with many tool results
    let total_turns = 400;

    let mut events: Vec<Event> = Vec::new();
    let mut seq = 0i32;
    let mut tool_call_counter = 0usize;

    for i in 0..total_turns {
        if i == needle_position {
            // Plant the needle as a user message
            events.push(make_input_event(
                session_id,
                format!(
                    "By the way, I set the {} to {} in the .env file",
                    key_name, key_value
                ),
                seq,
            ));
            seq += 1;
            events.push(make_output_event(
                session_id,
                "Got it, I've noted that configuration.".to_string(),
                seq,
            ));
            seq += 1;
        } else if i % 5 == 0 {
            // Every 5th turn: user asks to read a file, assistant calls tool, tool returns large result
            let filename = generate_realistic_filename(i + variant);
            events.push(make_input_event(
                session_id,
                format!("Can you read {} and tell me what's in it?", filename),
                seq,
            ));
            seq += 1;

            let tool_call_id = format!("call_needle_{}_{}", variant, tool_call_counter);
            tool_call_counter += 1;
            events.push(make_output_event_with_tool_call(
                session_id,
                "",
                &tool_call_id,
                "read_file",
                serde_json::json!({"path": filename}),
                seq,
            ));
            seq += 1;

            events.push(make_tool_event(
                session_id,
                "read_file",
                &tool_call_id,
                generate_large_file_content(i + variant),
                seq,
            ));
            seq += 1;

            events.push(make_output_event(
                session_id,
                generate_file_summary_response(i),
                seq,
            ));
            seq += 1;
        } else if i % 2 == 0 {
            events.push(make_input_event(
                session_id,
                generate_filler_user_message(i),
                seq,
            ));
            seq += 1;
        } else {
            events.push(make_output_event(
                session_id,
                generate_filler_assistant_message(i),
                seq,
            ));
            seq += 1;
        }
    }

    DatasetRecord {
        id: format!("needle_basic_{}", variant),
        description: format!(
            "Find {} value planted at turn {} in {} turn conversation with large tool results",
            key_name, needle_position, total_turns
        ),
        task: format!(
            "What is the {} we configured earlier in the conversation?",
            key_name
        ),
        input: Input { events },
        expectations: vec![
            format!("Response mentions the value: {}", key_value),
            format!("Response correctly identifies it as the {}", key_name),
        ],
        meta: Meta {
            scenario_type: Some("needle_in_haystack".to_string()),
            planted_at: Some(needle_position),
            planted_key: Some(key_name.to_string()),
            planted_value: Some(key_value.to_string()),
            planted_info: vec![],
        },
    }
}

/// Generate a multi-hop scenario requiring synthesis from multiple points
fn generate_multi_hop_scenario(variant: usize) -> DatasetRecord {
    let session_id = SessionId::new();
    let mut events: Vec<Event> = Vec::new();
    let mut seq = 0i32;
    let mut tool_call_counter = 0usize;
    let total_turns = 350;

    let service_name = "UserAuthService";
    let auth_method = "OAuth 2.0";
    let storage = "Redis";
    let port = "8443";

    // Plant information at early positions so they get trimmed
    let plants = [
        (
            15,
            format!("Let's call the authentication service {}", service_name),
        ),
        (
            40,
            format!(
                "{} will use {} for authentication flow",
                service_name, auth_method
            ),
        ),
        (
            70,
            format!("We'll cache {} sessions in {}", service_name, storage),
        ),
        (100, format!("{} should run on port {}", service_name, port)),
    ];

    for i in 0..total_turns {
        let plant = plants.iter().find(|(pos, _)| *pos == i);

        if let Some((_, content)) = plant {
            events.push(make_input_event(session_id, content.clone(), seq));
            seq += 1;
            events.push(make_output_event(
                session_id,
                "Understood, I've noted that configuration decision.".to_string(),
                seq,
            ));
            seq += 1;
        } else if i % 4 == 0 {
            // Every 4th turn: file read with large tool result
            let filename = generate_realistic_filename(i + variant + 1000);
            events.push(make_input_event(
                session_id,
                format!("Let me check {}", filename),
                seq,
            ));
            seq += 1;

            let tool_call_id = format!("call_multihop_{}_{}", variant, tool_call_counter);
            tool_call_counter += 1;
            events.push(make_output_event_with_tool_call(
                session_id,
                "",
                &tool_call_id,
                "read_file",
                serde_json::json!({"path": filename}),
                seq,
            ));
            seq += 1;

            events.push(make_tool_event(
                session_id,
                "read_file",
                &tool_call_id,
                generate_large_file_content(i + variant + 1000),
                seq,
            ));
            seq += 1;

            events.push(make_output_event(
                session_id,
                generate_file_summary_response(i + 1000),
                seq,
            ));
            seq += 1;
        } else if i % 2 == 0 {
            events.push(make_input_event(
                session_id,
                generate_filler_user_message(i),
                seq,
            ));
            seq += 1;
        } else {
            events.push(make_output_event(
                session_id,
                generate_filler_assistant_message(i),
                seq,
            ));
            seq += 1;
        }
    }

    DatasetRecord {
        id: format!("multi_hop_{}", variant),
        description: format!(
            "Synthesize service configuration from 4 points across {} turn conversation",
            total_turns
        ),
        task: format!(
            "Summarize the complete configuration for {}: what authentication method, caching, and port?",
            service_name
        ),
        input: Input { events },
        expectations: vec![
            format!("Response mentions authentication method: {}", auth_method),
            format!("Response mentions caching solution: {}", storage),
            format!("Response mentions port: {}", port),
            "Response synthesizes all three pieces of information".to_string(),
        ],
        meta: Meta {
            scenario_type: Some("multi_hop".to_string()),
            planted_info: plants
                .iter()
                .map(|(idx, content)| PlantedInfo {
                    event_index: *idx,
                    key: format!("info_{}", idx),
                    value: content.clone(),
                })
                .collect(),
            ..Default::default()
        },
    }
}

/// Generate a cumulative scenario tracking changes over time
fn generate_cumulative_scenario(variant: usize) -> DatasetRecord {
    let session_id = SessionId::new();
    let mut events: Vec<Event> = Vec::new();
    let mut seq = 0i32;
    let mut tool_call_counter = 0usize;

    let function_versions = [
        (5, "fn calculate(x: i32) -> i32 { x }"),
        (30, "fn calculate(x: i32) -> i32 { x * 2 }"),
        (60, "fn calculate(x: i32, y: i32) -> i32 { (x + y) * 2 }"),
        (
            90,
            "fn calculate(x: i32, y: i32) -> i32 { ((x + y) * 2).max(0) }",
        ),
        (
            120,
            "fn calculate(x: i32, y: i32, factor: i32) -> i32 { ((x + y) * factor).max(0) }",
        ),
    ];

    let total_turns = 380;

    for i in 0..total_turns {
        let version = function_versions.iter().find(|(pos, _)| *pos == i);

        if let Some((_, code)) = version {
            events.push(make_input_event(
                session_id,
                format!("Update the function to: ```rust\n{}\n```", code),
                seq,
            ));
            seq += 1;
            events.push(make_output_event(
                session_id,
                "Done, I've updated the calculate function.".to_string(),
                seq,
            ));
            seq += 1;
        } else if i % 4 == 0 {
            // Every 4th turn: large tool result
            let tool_call_id = format!("call_cumul_{}_{}", variant, tool_call_counter);
            tool_call_counter += 1;
            let filename = generate_realistic_filename(i + variant + 2000);

            events.push(make_input_event(
                session_id,
                format!("Read {}", filename),
                seq,
            ));
            seq += 1;

            events.push(make_output_event_with_tool_call(
                session_id,
                "",
                &tool_call_id,
                "read_file",
                serde_json::json!({"path": filename}),
                seq,
            ));
            seq += 1;

            events.push(make_tool_event(
                session_id,
                "read_file",
                &tool_call_id,
                generate_large_file_content(i + variant + 2000),
                seq,
            ));
            seq += 1;

            events.push(make_output_event(
                session_id,
                generate_file_summary_response(i + 2000),
                seq,
            ));
            seq += 1;
        } else if i % 3 == 0 {
            events.push(make_input_event(
                session_id,
                generate_filler_user_message(i),
                seq,
            ));
            seq += 1;
        } else {
            events.push(make_output_event(
                session_id,
                generate_filler_assistant_message(i),
                seq,
            ));
            seq += 1;
        }
    }

    let final_version = function_versions.last().unwrap().1;

    DatasetRecord {
        id: format!("cumulative_{}", variant),
        description: format!(
            "Track cumulative changes to a function across {} turn conversation",
            total_turns
        ),
        task: "What is the current/final version of the calculate function? Show the complete function signature and body.".to_string(),
        input: Input { events },
        expectations: vec![
            "Response shows the FINAL version with factor parameter".to_string(),
            "Response includes .max(0) call".to_string(),
            format!("Response shows: {}", final_version),
        ],
        meta: Meta {
            scenario_type: Some("cumulative".to_string()),
            planted_info: function_versions
                .iter()
                .map(|(idx, code)| PlantedInfo {
                    event_index: *idx,
                    key: format!("version_{}", idx),
                    value: code.to_string(),
                })
                .collect(),
            ..Default::default()
        },
    }
}

/// Generate a scenario where a decision changes multiple times
fn generate_final_decision_scenario(variant: usize) -> DatasetRecord {
    let session_id = SessionId::new();

    let decision_topics = [
        (
            "database",
            "What database did we finally decide to use?",
            vec![
                ("PostgreSQL", "Let's use PostgreSQL for the database"),
                ("MySQL", "Actually, let's switch to MySQL instead"),
                ("MongoDB", "On second thought, MongoDB would be better"),
                (
                    "PostgreSQL",
                    "After more research, let's go back to PostgreSQL",
                ),
                ("CockroachDB", "Final decision: we'll use CockroachDB"),
            ],
        ),
        (
            "framework",
            "What web framework did we finally decide to use?",
            vec![
                ("Express", "Let's start with Express for the API"),
                ("Fastify", "Actually, Fastify would be faster"),
                ("Hono", "Let's try Hono for edge compatibility"),
                ("Express", "Back to Express for the ecosystem"),
                ("Axum", "Final decision: we'll use Axum in Rust"),
            ],
        ),
        (
            "cloud",
            "What cloud provider did we finally decide to use?",
            vec![
                ("AWS", "Let's deploy on AWS"),
                ("GCP", "Actually, GCP has better ML support"),
                ("Azure", "The client prefers Azure"),
                ("AWS", "AWS pricing is better for our scale"),
                ("Fly.io", "Final decision: Fly.io for simplicity"),
            ],
        ),
    ];

    let (topic, task, choices) = &decision_topics[variant % decision_topics.len()];
    let total_turns = 360;
    let mut events: Vec<Event> = Vec::new();
    let mut seq = 0i32;
    let mut tool_call_counter = 0usize;

    // Spread decisions across the conversation, earlier ones will be trimmed
    let positions: Vec<usize> = (0..choices.len())
        .map(|i| 10 + i * (total_turns / (choices.len() + 1)))
        .collect();

    let decisions: Vec<(usize, &str, &str)> = positions
        .iter()
        .zip(choices.iter())
        .map(|(pos, (name, stmt))| (*pos, *name, *stmt))
        .collect();

    for i in 0..total_turns {
        let decision = decisions.iter().find(|(pos, _, _)| *pos == i);

        if let Some((_, _name, statement)) = decision {
            events.push(make_input_event(session_id, statement.to_string(), seq));
            seq += 1;
            events.push(make_output_event(
                session_id,
                "Understood, I'll update the configuration.".to_string(),
                seq,
            ));
            seq += 1;
        } else if i % 4 == 0 {
            // Large tool results
            let tool_call_id = format!("call_decision_{}_{}", variant, tool_call_counter);
            tool_call_counter += 1;
            let filename = generate_realistic_filename(i + variant + 3000);

            events.push(make_input_event(
                session_id,
                format!("Check {}", filename),
                seq,
            ));
            seq += 1;

            events.push(make_output_event_with_tool_call(
                session_id,
                "",
                &tool_call_id,
                "read_file",
                serde_json::json!({"path": filename}),
                seq,
            ));
            seq += 1;

            events.push(make_tool_event(
                session_id,
                "read_file",
                &tool_call_id,
                generate_large_file_content(i + variant + 3000),
                seq,
            ));
            seq += 1;

            events.push(make_output_event(
                session_id,
                generate_file_summary_response(i + 3000),
                seq,
            ));
            seq += 1;
        } else if i % 2 == 0 {
            events.push(make_input_event(
                session_id,
                generate_filler_user_message(i + variant),
                seq,
            ));
            seq += 1;
        } else {
            events.push(make_output_event(
                session_id,
                generate_filler_assistant_message(i + variant),
                seq,
            ));
            seq += 1;
        }
    }

    let final_choice = decisions.last().unwrap().1;

    DatasetRecord {
        id: format!("final_decision_{}_{}", topic, variant),
        description: format!(
            "Decision about {} changes {} times across {} turns",
            topic,
            choices.len(),
            total_turns
        ),
        task: task.to_string(),
        input: Input { events },
        expectations: vec![
            format!("Response identifies the FINAL choice: {}", final_choice),
            "Response does not confuse earlier decisions with the final one".to_string(),
        ],
        meta: Meta {
            scenario_type: Some("final_decision".to_string()),
            planted_info: decisions
                .iter()
                .map(|(idx, name, statement)| PlantedInfo {
                    event_index: *idx,
                    key: format!("decision_{}", idx),
                    value: format!("{}: {}", name, statement),
                })
                .collect(),
            ..Default::default()
        },
    }
}

/// Generate a scenario asking for timeline of decision changes
fn generate_decision_timeline_scenario(variant: usize) -> DatasetRecord {
    let session_id = SessionId::new();

    let timeline_topics = [
        (
            "api_version",
            "List all the API versions we went through in order.",
            vec![
                ("v1", "Starting with v1"),
                ("v2", "Bump to v2"),
                ("v2.1", "Patch v2.1"),
                ("v3", "Major v3"),
            ],
        ),
        (
            "sprint",
            "List all the sprint names we completed in order.",
            vec![
                ("Alpha", "Sprint Alpha"),
                ("Beta", "Sprint Beta"),
                ("Gamma", "Sprint Gamma"),
                ("Delta", "Sprint Delta"),
                ("Epsilon", "Sprint Epsilon"),
            ],
        ),
        (
            "release",
            "List all the release codenames in order.",
            vec![
                ("Falcon", "Release Falcon"),
                ("Griffin", "Release Griffin"),
                ("Hydra", "Release Hydra"),
            ],
        ),
    ];

    let (topic, task, versions) = &timeline_topics[variant % timeline_topics.len()];
    let total_turns = 340;
    let mut events: Vec<Event> = Vec::new();
    let mut seq = 0i32;
    let mut tool_call_counter = 0usize;

    let positions: Vec<usize> = (0..versions.len())
        .map(|i| 15 + i * (total_turns / (versions.len() + 1)))
        .collect();

    let version_list: Vec<(usize, &str, &str)> = positions
        .iter()
        .zip(versions.iter())
        .map(|(pos, (ver, stmt))| (*pos, *ver, *stmt))
        .collect();

    for i in 0..total_turns {
        let version = version_list.iter().find(|(pos, _, _)| *pos == i);

        if let Some((_, ver, statement)) = version {
            events.push(make_input_event(
                session_id,
                format!("{} - {}", statement, ver),
                seq,
            ));
            seq += 1;
            events.push(make_output_event(
                session_id,
                format!("Updated to {}.", ver),
                seq,
            ));
            seq += 1;
        } else if i % 4 == 0 {
            // Large tool results
            let tool_call_id = format!("call_timeline_{}_{}", variant, tool_call_counter);
            tool_call_counter += 1;
            let filename = generate_realistic_filename(i + variant + 4000);

            events.push(make_input_event(
                session_id,
                format!("Show me {}", filename),
                seq,
            ));
            seq += 1;

            events.push(make_output_event_with_tool_call(
                session_id,
                "",
                &tool_call_id,
                "read_file",
                serde_json::json!({"path": filename}),
                seq,
            ));
            seq += 1;

            events.push(make_tool_event(
                session_id,
                "read_file",
                &tool_call_id,
                generate_large_file_content(i + variant + 4000),
                seq,
            ));
            seq += 1;

            events.push(make_output_event(
                session_id,
                generate_file_summary_response(i + 4000),
                seq,
            ));
            seq += 1;
        } else if i % 2 == 0 {
            events.push(make_input_event(
                session_id,
                generate_filler_user_message(i + variant),
                seq,
            ));
            seq += 1;
        } else {
            events.push(make_output_event(
                session_id,
                generate_filler_assistant_message(i + variant),
                seq,
            ));
            seq += 1;
        }
    }

    let version_names: Vec<String> = versions.iter().map(|(v, _)| v.to_string()).collect();

    DatasetRecord {
        id: format!("timeline_{}_{}", topic, variant),
        description: format!(
            "Recreate timeline of {} changes across {} turns",
            topic, total_turns
        ),
        task: task.to_string(),
        input: Input { events },
        expectations: vec![
            format!("Response lists all versions: {}", version_names.join(", ")),
            format!(
                "Response lists them in correct chronological order: {}",
                version_names.join(" → ")
            ),
        ],
        meta: Meta {
            scenario_type: Some("decision_timeline".to_string()),
            planted_info: version_list
                .iter()
                .map(|(idx, ver, statement)| PlantedInfo {
                    event_index: *idx,
                    key: format!("version_{}", idx),
                    value: format!("{}: {}", ver, statement),
                })
                .collect(),
            ..Default::default()
        },
    }
}

/// Generate a scenario with same tool called multiple times
fn generate_tool_disambiguation_scenario(variant: usize) -> DatasetRecord {
    let session_id = SessionId::new();

    let tool_scenarios = [
        (
            "config_port",
            "What is the current port in config.json?",
            "8080",
            vec![
                ("config.json", r#"{"port": 3000, "host": "localhost"}"#),
                ("users.json", r#"[{"id": 1, "name": "Alice"}]"#),
                ("config.json", r#"{"port": 8080, "host": "0.0.0.0"}"#),
            ],
        ),
        (
            "env_debug",
            "What is the current DEBUG value in .env?",
            "false",
            vec![
                (".env", "DEBUG=true\nPORT=3000"),
                ("package.json", r#"{"name": "app", "version": "1.0"}"#),
                (".env", "DEBUG=false\nPORT=8080"),
                ("README.md", "# App\nDocumentation here"),
            ],
        ),
        (
            "db_host",
            "What is the current database host in database.yml?",
            "prod-db.example.com",
            vec![
                ("database.yml", "host: localhost\nport: 5432"),
                ("database.yml", "host: staging-db.example.com\nport: 5432"),
                ("schema.sql", "CREATE TABLE users (id INT);"),
                ("database.yml", "host: prod-db.example.com\nport: 5432"),
            ],
        ),
    ];

    let (name, task, expected, target_tool_calls) = &tool_scenarios[variant % tool_scenarios.len()];
    let total_turns = 320;
    let mut events: Vec<Event> = Vec::new();
    let mut seq = 0i32;
    let mut tool_call_counter = 0usize;

    // Spread the target file reads across the conversation
    let positions: Vec<usize> = (0..target_tool_calls.len())
        .map(|i| 20 + i * (total_turns / (target_tool_calls.len() + 1)))
        .collect();

    let calls: Vec<(usize, &str, &str)> = positions
        .iter()
        .zip(target_tool_calls.iter())
        .map(|(pos, (file, content))| (*pos, *file, *content))
        .collect();

    for i in 0..total_turns {
        let tool_call = calls.iter().find(|(pos, _, _)| *pos == i);

        if let Some((_, filename, content)) = tool_call {
            // User asks to read file
            events.push(make_input_event(
                session_id,
                format!("Read the file {}", filename),
                seq,
            ));
            seq += 1;

            // Assistant calls the tool
            let tool_call_id = format!("call_disambig_{}_{}", variant, tool_call_counter);
            tool_call_counter += 1;
            events.push(make_output_event_with_tool_call(
                session_id,
                "",
                &tool_call_id,
                "read_file",
                serde_json::json!({"filename": filename}),
                seq,
            ));
            seq += 1;

            // Tool result
            events.push(make_tool_event(
                session_id,
                "read_file",
                &tool_call_id,
                format!("Contents of {}:\n{}", filename, content),
                seq,
            ));
            seq += 1;

            // Assistant response
            events.push(make_output_event(
                session_id,
                format!("I've read {}.", filename),
                seq,
            ));
            seq += 1;
        } else if i % 4 == 0 {
            // Large filler tool results (different files)
            let tool_call_id = format!("call_disambig_filler_{}_{}", variant, tool_call_counter);
            tool_call_counter += 1;
            let filename = generate_realistic_filename(i + variant + 5000);

            events.push(make_input_event(
                session_id,
                format!("Check {}", filename),
                seq,
            ));
            seq += 1;

            events.push(make_output_event_with_tool_call(
                session_id,
                "",
                &tool_call_id,
                "read_file",
                serde_json::json!({"path": filename}),
                seq,
            ));
            seq += 1;

            events.push(make_tool_event(
                session_id,
                "read_file",
                &tool_call_id,
                generate_large_file_content(i + variant + 5000),
                seq,
            ));
            seq += 1;

            events.push(make_output_event(
                session_id,
                generate_file_summary_response(i + 5000),
                seq,
            ));
            seq += 1;
        } else if i % 2 == 0 {
            events.push(make_input_event(
                session_id,
                generate_filler_user_message(i + variant),
                seq,
            ));
            seq += 1;
        } else {
            events.push(make_output_event(
                session_id,
                generate_filler_assistant_message(i + variant),
                seq,
            ));
            seq += 1;
        }
    }

    DatasetRecord {
        id: format!("tool_disambig_{}_{}", name, variant),
        description: format!(
            "Same file read multiple times across {} turns, must use most recent",
            total_turns
        ),
        task: format!("{} (Use the most recent read)", task),
        input: Input { events },
        expectations: vec![
            format!(
                "Response uses the value from the MOST RECENT read: {}",
                expected
            ),
            "Response does not confuse earlier file reads with the latest one".to_string(),
        ],
        meta: Meta {
            scenario_type: Some("tool_result_disambiguation".to_string()),
            planted_info: calls
                .iter()
                .map(|(idx, filename, content)| PlantedInfo {
                    event_index: *idx,
                    key: format!("{}_{}", filename, idx),
                    value: content.to_string(),
                })
                .collect(),
            ..Default::default()
        },
    }
}

// =============================================================================
// Helper Functions for Generating Content
// =============================================================================

/// Generate realistic file paths
fn generate_realistic_filename(seed: usize) -> String {
    let paths = [
        "src/lib.rs",
        "src/main.rs",
        "src/config.rs",
        "src/database/mod.rs",
        "src/database/queries.rs",
        "src/api/handlers.rs",
        "src/api/routes.rs",
        "src/models/user.rs",
        "src/models/session.rs",
        "src/utils/helpers.rs",
        "src/middleware/auth.rs",
        "tests/integration_test.rs",
        "tests/unit_tests.rs",
        "Cargo.toml",
        "package.json",
        "tsconfig.json",
        "docker-compose.yml",
        "Dockerfile",
        ".env.example",
        "README.md",
        "CHANGELOG.md",
        "scripts/deploy.sh",
        "scripts/setup.sh",
        "migrations/001_create_users.sql",
        "migrations/002_add_sessions.sql",
    ];
    paths[seed % paths.len()].to_string()
}

/// Generate a summary response after reading a file
fn generate_file_summary_response(seed: usize) -> String {
    let responses = [
        "I've read the file. It contains the main application configuration with database settings, API endpoints, and feature flags. The structure follows the standard pattern we've been using.",
        "This file contains the core business logic for handling user requests. I can see the validation rules, data transformation functions, and error handling patterns we discussed earlier.",
        "The file shows our database schema and query definitions. It includes indexes, foreign keys, and the audit columns we added for compliance.",
        "I've reviewed this configuration file. It has the environment-specific settings for development, staging, and production deployments.",
        "This contains our API route definitions and middleware configuration. The authentication flow and rate limiting settings are all present.",
        "The file includes utility functions for data parsing, validation, and formatting. These are used across multiple modules in the codebase.",
        "I can see the test fixtures and mock data definitions. The test coverage includes unit tests, integration tests, and end-to-end scenarios.",
        "This file defines the data models and their relationships. The type definitions match our database schema and API contracts.",
    ];
    responses[seed % responses.len()].to_string()
}

/// Generate filler user messages (longer to fill token budget faster)
fn generate_filler_user_message(seed: usize) -> String {
    let templates = [
        "Can you help me refactor the logging module? I've been looking at the current implementation and it seems like we have a lot of duplicated code across different components. The logger is being instantiated in multiple places with similar configurations, and I think we could benefit from a centralized logging factory or a dependency injection approach. Also, the log levels seem inconsistent - some modules use debug for what should be info, and vice versa.",
        "What do you think about using async/await here? I'm concerned about the performance implications, especially since we're dealing with multiple concurrent database connections and API calls. The current synchronous implementation is causing blocking issues when we have more than 50 concurrent users. I've been reading about tokio and its runtime, but I'm not sure if the migration effort would be worth it for our use case.",
        "Let's add some error handling to this function. Right now it just panics on any error, which is causing issues in production. We should probably use Result types and propagate errors up the call stack. I'm also thinking we might need custom error types to distinguish between recoverable and unrecoverable errors, and maybe add some retry logic for transient failures.",
        "Should we split this into separate files? The current module is over 2000 lines and it's becoming difficult to navigate. I'm thinking we could have separate files for the data structures, the business logic, the database operations, and the API handlers. This would also make it easier for multiple team members to work on different parts without merge conflicts.",
        "I'm getting a type error on line 42 that says 'expected struct Vec, found &[T]'. I've tried adding .to_vec() but then I get a lifetime error. The function signature takes a &[T] but internally I need to store it in a struct that owns the data. Should I change the function to take ownership, or is there a better way to handle this with Cow<[T]>?",
        "Can you explain how this iterator works? I see it's using filter_map with a closure that returns Option, and then there's a flat_map that seems to be unpacking nested structures. The chain of transformations is confusing me, especially the part where we collect into a HashMap and then iterate again. Is there a more readable way to write this?",
        "Let's add tests for the edge cases. Currently we only have happy path tests, but I've identified several scenarios that aren't covered: empty input, maximum length input, unicode characters, concurrent modifications, timeout scenarios, and partial failures. We should also add some property-based tests using proptest to catch edge cases we haven't thought of.",
        "What's the best way to handle this edge case? When the user submits a form with special characters in the name field, the validation passes but the database insert fails because of encoding issues. We need to either sanitize the input, change the database column encoding, or return a more helpful error message. I'm leaning towards input sanitization but I'm worried about data loss.",
        "Should we use a hashmap or a vec here? The current implementation uses a Vec and does linear search, which is O(n) for lookups. With a HashMap we'd get O(1) lookups, but we'd lose ordering and the memory overhead might be significant for our small datasets. The data size is typically 10-50 items, but occasionally goes up to 1000. What's the crossover point where HashMap becomes worth it?",
        "Can you review this implementation? I've added the new feature but I'm not confident about the error handling strategy. Also, I'm not sure if the mutex lock scope is correct - it might be held too long and cause contention. The tests pass but I haven't done any load testing yet. Can you spot any potential race conditions or performance issues?",
    ];
    templates[seed % templates.len()].to_string()
}

/// Generate filler assistant messages (longer to fill token budget faster)
fn generate_filler_assistant_message(seed: usize) -> String {
    let templates = [
        "That's a good approach for the logging refactor. I'll create a LoggerFactory that uses the builder pattern to configure loggers. Each module can request a logger with its name, and the factory will handle the configuration centrally. I'll also add a LogLevel enum that enforces consistency, and create a macro for the common logging patterns so we reduce boilerplate. The factory will support both sync and async logging backends.",
        "I'll refactor this to be more maintainable by extracting the core logic into pure functions that are easy to test. The side effects like database writes and API calls will be pushed to the boundaries. I'm going to use the repository pattern for data access and introduce interfaces so we can easily mock dependencies in tests. This should make the code more modular and easier to understand.",
        "Here's an updated version with better error handling. I've introduced a custom Error enum with variants for each failure mode: NetworkError, DatabaseError, ValidationError, and InternalError. Each variant carries context about what went wrong. I've also added the thiserror derive macro for better error messages, and implemented From traits so errors from external libraries are automatically converted.",
        "Good idea to split this into separate files. I've created a directory structure with mod.rs for the public interface, types.rs for data structures, logic.rs for business rules, db.rs for database operations, and handlers.rs for API endpoints. Each file has its own tests at the bottom. I've also added a prelude.rs that re-exports commonly used items for convenience.",
        "The error is due to a type mismatch between owned and borrowed data. Since you need to store the data in a struct, the cleanest solution is to change the function signature to take impl Into<Vec<T>> which accepts both Vec<T> and &[T] (via .into()). Alternatively, you could use Cow<'a, [T]> if you want to avoid allocation when the caller already has a slice and doesn't need ownership.",
        "This iterator chain uses a functional approach to transform data. The filter_map removes None values while unwrapping Some values in one step. The flat_map then unpacks nested iterators - it's like map followed by flatten. The collect into HashMap groups items by key, and the second iteration processes each group. For readability, I'd suggest breaking it into named intermediate variables or extracting helper functions.",
        "I've added comprehensive test coverage for all the edge cases you mentioned. For empty input, I'm testing that we return an empty result rather than erroring. The max length test uses a 10MB string to verify we don't blow up memory. Unicode tests cover emoji, RTL text, and combining characters. I've added a stress test that spawns 100 concurrent tasks to verify thread safety, and timeout tests that use tokio::time::pause() to control time.",
        "For this edge case with special characters, I recommend a multi-layered approach. First, validate input at the API boundary using a whitelist of allowed characters. Second, escape any remaining special characters before database insertion using parameterized queries. Third, add a database constraint to catch anything that slips through. I'll also add a clear error message that tells the user which characters are not allowed.",
        "For collections of 10-50 items, Vec with linear search is actually faster than HashMap due to cache locality - the overhead of hashing and handling collisions isn't worth it. The crossover point is typically around 20-30 items for string keys. However, at 1000 items, HashMap is definitely better. I suggest using a SmallVec or ArrayVec for the common case, with a fallback to HashMap when size exceeds a threshold of 100.",
        "I've reviewed the implementation and found a few issues. First, the mutex lock on line 45 is held across an await point, which will cause the lock to be held for the duration of the async operation - use tokio::sync::Mutex instead. Second, there's a potential race condition between the read on line 52 and the write on line 58 - another thread could modify the data in between. I recommend using a read-write lock and making the read-modify-write atomic.",
    ];
    templates[seed % templates.len()].to_string()
}

/// Generate filler file content for tool results (large to fill token budget)
/// Generate large file content for tool results (~1500-3000 tokens each)
/// This is the key function for making scenarios exceed the context budget
fn generate_large_file_content(seed: usize) -> String {
    let templates = [
        r#"use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Configuration manager for application settings.
///
/// This struct provides thread-safe access to configuration values
/// that can be loaded from multiple sources (files, environment, etc.)
/// and updated at runtime without requiring a restart.
#[derive(Debug, Clone)]
pub struct Config {
    values: Arc<RwLock<HashMap<String, ConfigValue>>>,
    watchers: Arc<RwLock<Vec<Box<dyn ConfigWatcher>>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConfigValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Array(Vec<ConfigValue>),
    Object(HashMap<String, ConfigValue>),
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Key not found: {0}")]
    KeyNotFound(String),
    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

impl Config {
    pub fn new() -> Self {
        Self {
            values: Arc::new(RwLock::new(HashMap::new())),
            watchers: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn get<T: FromConfigValue>(&self, key: &str) -> Result<T, ConfigError> {
        let values = self.values.read().unwrap();
        let value = values.get(key).ok_or_else(|| ConfigError::KeyNotFound(key.to_string()))?;
        T::from_config_value(value.clone())
    }

    pub fn set(&self, key: &str, value: ConfigValue) {
        let mut values = self.values.write().unwrap();
        values.insert(key.to_string(), value.clone());
        drop(values);
        self.notify_watchers(key, &value);
    }

    fn notify_watchers(&self, key: &str, value: &ConfigValue) {
        let watchers = self.watchers.read().unwrap();
        for watcher in watchers.iter() {
            watcher.on_config_change(key, value);
        }
    }
}

pub trait FromConfigValue: Sized {
    fn from_config_value(value: ConfigValue) -> Result<Self, ConfigError>;
}

pub trait ConfigWatcher: Send + Sync {
    fn on_config_change(&self, key: &str, value: &ConfigValue);
}"#,
        r#"mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{sleep, Duration};

    /// Test fixture for setting up common test state.
    /// Provides helper methods for creating test instances
    /// and asserting on expected outcomes.
    struct TestFixture {
        db: MockDatabase,
        cache: MockCache,
        logger: MockLogger,
    }

    impl TestFixture {
        fn new() -> Self {
            Self {
                db: MockDatabase::new(),
                cache: MockCache::new(),
                logger: MockLogger::new(),
            }
        }

        fn with_data(mut self, data: Vec<TestRecord>) -> Self {
            for record in data {
                self.db.insert(record);
            }
            self
        }
    }

    #[test]
    fn test_basic_crud_operations() {
        let fixture = TestFixture::new();
        let service = Service::new(fixture.db, fixture.cache);

        // Test create
        let record = TestRecord { id: 1, name: "test".to_string() };
        let result = service.create(record.clone());
        assert!(result.is_ok());

        // Test read
        let fetched = service.get(1).unwrap();
        assert_eq!(fetched.name, "test");

        // Test update
        let updated = TestRecord { id: 1, name: "updated".to_string() };
        service.update(updated).unwrap();
        let fetched = service.get(1).unwrap();
        assert_eq!(fetched.name, "updated");

        // Test delete
        service.delete(1).unwrap();
        assert!(service.get(1).is_err());
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        let counter = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();

        for _ in 0..100 {
            let counter = Arc::clone(&counter);
            handles.push(tokio::spawn(async move {
                counter.fetch_add(1, Ordering::SeqCst);
                sleep(Duration::from_millis(10)).await;
            }));
        }

        for handle in handles {
            handle.await.unwrap();
        }

        assert_eq!(counter.load(Ordering::SeqCst), 100);
    }

    #[test]
    fn test_error_handling() {
        let fixture = TestFixture::new();
        let service = Service::new(fixture.db, fixture.cache);

        // Test not found error
        let result = service.get(999);
        assert!(matches!(result, Err(ServiceError::NotFound(_))));

        // Test validation error
        let invalid = TestRecord { id: -1, name: "".to_string() };
        let result = service.create(invalid);
        assert!(matches!(result, Err(ServiceError::ValidationError(_))));
    }
}"#,
        r#"use async_trait::async_trait;
use std::time::Duration;
use tokio::time::timeout;

/// Process input data through a pipeline of transformations.
///
/// This function applies a series of transformations to the input,
/// with configurable timeout, retry logic, and error handling.
///
/// # Arguments
/// * `input` - The raw input string to process
/// * `config` - Processing configuration options
///
/// # Returns
/// * `Ok(ProcessedOutput)` - Successfully processed result
/// * `Err(ProcessError)` - Processing failed after all retries
///
/// # Example
/// ```rust
/// let config = ProcessConfig::default();
/// let result = process("hello world", &config).await?;
/// assert_eq!(result.transformed, "HELLO WORLD");
/// ```
pub async fn process(input: &str, config: &ProcessConfig) -> Result<ProcessedOutput, ProcessError> {
    let mut last_error = None;

    for attempt in 0..config.max_retries {
        match timeout(config.timeout, process_inner(input)).await {
            Ok(Ok(result)) => return Ok(result),
            Ok(Err(e)) => {
                log::warn!("Process attempt {} failed: {}", attempt + 1, e);
                last_error = Some(e);
            }
            Err(_) => {
                log::warn!("Process attempt {} timed out", attempt + 1);
                last_error = Some(ProcessError::Timeout);
            }
        }

        if attempt < config.max_retries - 1 {
            let backoff = Duration::from_millis(100 * 2u64.pow(attempt as u32));
            tokio::time::sleep(backoff).await;
        }
    }

    Err(last_error.unwrap_or(ProcessError::Unknown))
}

async fn process_inner(input: &str) -> Result<ProcessedOutput, ProcessError> {
    // Validate input
    if input.is_empty() {
        return Err(ProcessError::EmptyInput);
    }
    if input.len() > MAX_INPUT_LENGTH {
        return Err(ProcessError::InputTooLong(input.len()));
    }

    // Transform the input
    let normalized = input.trim().to_lowercase();
    let words: Vec<&str> = normalized.split_whitespace().collect();
    let transformed = words.join(" ").to_uppercase();

    // Build output
    Ok(ProcessedOutput {
        original: input.to_string(),
        transformed,
        word_count: words.len(),
        char_count: input.chars().count(),
        processing_time_ms: 0, // Would be set by actual timing
    })
}

const MAX_INPUT_LENGTH: usize = 1_000_000;

#[derive(Debug, Clone)]
pub struct ProcessConfig {
    pub max_retries: u32,
    pub timeout: Duration,
    pub validate_utf8: bool,
}

impl Default for ProcessConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            timeout: Duration::from_secs(30),
            validate_utf8: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProcessedOutput {
    pub original: String,
    pub transformed: String,
    pub word_count: usize,
    pub char_count: usize,
    pub processing_time_ms: u64,
}"#,
        r#"//! Database connection pool and query execution module.
//!
//! This module provides a high-performance connection pool for PostgreSQL
//! databases with support for prepared statements, transactions, and
//! automatic retry on transient errors.
//!
//! # Features
//! - Connection pooling with configurable size limits
//! - Prepared statement caching for query performance
//! - Automatic reconnection on connection drops
//! - Transaction support with savepoints
//! - Query timeout and cancellation
//! - Metrics and monitoring hooks
//!
//! # Example
//! ```rust
//! let pool = ConnectionPool::new(config).await?;
//! let conn = pool.acquire().await?;
//! let rows = conn.query("SELECT * FROM users WHERE id = $1", &[&user_id]).await?;
//! ```

use deadpool_postgres::{Config, Pool, Runtime};
use tokio_postgres::{NoTls, Row};
use std::time::Duration;

pub struct ConnectionPool {
    pool: Pool,
    metrics: PoolMetrics,
}

impl ConnectionPool {
    pub async fn new(config: DatabaseConfig) -> Result<Self, PoolError> {
        let mut cfg = Config::new();
        cfg.host = Some(config.host);
        cfg.port = Some(config.port);
        cfg.dbname = Some(config.database);
        cfg.user = Some(config.username);
        cfg.password = Some(config.password);

        let pool = cfg.create_pool(Some(Runtime::Tokio1), NoTls)?;

        Ok(Self {
            pool,
            metrics: PoolMetrics::default(),
        })
    }

    pub async fn query(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Vec<Row>, QueryError> {
        let client = self.pool.get().await?;
        let rows = client.query(sql, params).await?;
        self.metrics.queries_executed.fetch_add(1, Ordering::Relaxed);
        Ok(rows)
    }

    pub async fn execute(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<u64, QueryError> {
        let client = self.pool.get().await?;
        let rows_affected = client.execute(sql, params).await?;
        self.metrics.queries_executed.fetch_add(1, Ordering::Relaxed);
        Ok(rows_affected)
    }

    pub async fn transaction<F, T>(&self, f: F) -> Result<T, TransactionError>
    where
        F: FnOnce(&Transaction) -> Result<T, TransactionError>,
    {
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        let result = f(&tx)?;
        tx.commit().await?;
        Ok(result)
    }
}

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub password: String,
    pub max_connections: u32,
    pub connection_timeout: Duration,
}"#,
        r#"/// Application constants and configuration defaults.
///
/// This module contains all compile-time constants and default values
/// used throughout the application. Values are organized by category
/// and documented with their purpose and valid ranges.

// =============================================================================
// Network Configuration
// =============================================================================

/// Maximum number of retry attempts for network operations
pub const MAX_RETRIES: u32 = 3;

/// Default timeout for network requests in milliseconds
pub const TIMEOUT_MS: u64 = 5000;

/// Maximum number of concurrent connections per host
pub const MAX_CONNECTIONS_PER_HOST: usize = 100;

/// Keep-alive interval for long-lived connections
pub const KEEP_ALIVE_INTERVAL_SECS: u64 = 30;

/// TCP nodelay setting (disable Nagle's algorithm)
pub const TCP_NODELAY: bool = true;

// =============================================================================
// Rate Limiting
// =============================================================================

/// Maximum requests per second per client
pub const RATE_LIMIT_RPS: u32 = 100;

/// Rate limit bucket size (burst capacity)
pub const RATE_LIMIT_BURST: u32 = 200;

/// Rate limit window duration in seconds
pub const RATE_LIMIT_WINDOW_SECS: u64 = 1;

// =============================================================================
// Caching
// =============================================================================

/// Default cache TTL in seconds
pub const CACHE_TTL_SECS: u64 = 300;

/// Maximum cache size in bytes
pub const MAX_CACHE_SIZE_BYTES: usize = 100 * 1024 * 1024; // 100 MB

/// Cache eviction check interval
pub const CACHE_EVICTION_INTERVAL_SECS: u64 = 60;

// =============================================================================
// Database
// =============================================================================

/// Maximum database connection pool size
pub const DB_POOL_MAX_SIZE: u32 = 20;

/// Minimum database connection pool size
pub const DB_POOL_MIN_SIZE: u32 = 5;

/// Database connection acquire timeout
pub const DB_CONNECTION_TIMEOUT_MS: u64 = 3000;

/// Maximum query execution time before timeout
pub const DB_QUERY_TIMEOUT_MS: u64 = 30000;

// =============================================================================
// Security
// =============================================================================

/// JWT token expiration time in seconds
pub const JWT_EXPIRATION_SECS: u64 = 3600;

/// Session cookie max age in seconds
pub const SESSION_MAX_AGE_SECS: u64 = 86400;

/// Password minimum length
pub const PASSWORD_MIN_LENGTH: usize = 12;

/// Maximum failed login attempts before lockout
pub const MAX_LOGIN_ATTEMPTS: u32 = 5;

/// Account lockout duration in seconds
pub const LOCKOUT_DURATION_SECS: u64 = 900;"#,
    ];
    templates[seed % templates.len()].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_synthetic_scenarios() {
        let records = generate_synthetic();

        // Should have 6 records (one of each type)
        assert_eq!(records.len(), 6);

        // Should have different types
        let types: Vec<_> = records
            .iter()
            .filter_map(|r| r.meta.scenario_type.as_ref())
            .collect();
        assert!(types.iter().any(|t| t.as_str() == "needle_in_haystack"));
        assert!(types.iter().any(|t| t.as_str() == "multi_hop"));
        assert!(types.iter().any(|t| t.as_str() == "cumulative"));
        assert!(types.iter().any(|t| t.as_str() == "final_decision"));
        assert!(types.iter().any(|t| t.as_str() == "decision_timeline"));
        assert!(types
            .iter()
            .any(|t| t.as_str() == "tool_result_disambiguation"));
    }

    #[test]
    fn test_needle_scenario_has_planted_info() {
        let record = generate_needle_scenario(0);

        assert!(record.meta.planted_at.is_some());
        assert!(record.meta.planted_value.is_some());
        assert!(!record.input.events.is_empty());
        assert!(!record.expectations.is_empty());
    }

    #[test]
    fn test_all_records_have_expectations() {
        let records = generate_synthetic();
        for record in &records {
            assert!(
                !record.expectations.is_empty(),
                "Record {} should have expectations",
                record.id
            );
        }
    }
}
