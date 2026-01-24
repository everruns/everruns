//! Synthetic scenario generation
//!
//! Generates DatasetRecord with events for eval datasets.

use crate::dataset::{DatasetRecord, Event, Input, Meta, PlantedInfo};

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
    // Fake secrets for evaluation scenarios - NOT real credentials
    let secrets = [
        ("API_KEY", "fake-api-key-abc123xyz789"),
        ("DATABASE_PASSWORD", "fake-db-password-42"),
        ("JWT_SECRET", "fake-jwt-secret-key"),
        ("ACCESS_KEY", "FAKE-ACCESS-KEY-EXAMPLE"),
        ("PAYMENT_KEY", "fake-payment-key-example"),
    ];

    let (key_name, key_value) = secrets[variant % secrets.len()];
    let needle_position = 15 + (variant * 7) % 30;
    let total_events = 150 + (variant * 20);

    let mut events = Vec::new();

    for i in 0..total_events {
        if i == needle_position {
            // Plant the needle
            events.push(Event::user(format!(
                "By the way, I set the {} to {} in the .env file",
                key_name, key_value
            )));
        } else if i % 2 == 0 {
            events.push(Event::user(generate_filler_user_message(i)));
        } else {
            events.push(Event::assistant(generate_filler_assistant_message(i)));
        }
    }

    DatasetRecord {
        id: format!("needle_basic_{}", variant),
        description: format!(
            "Find {} value planted at event {} in {} event conversation",
            key_name, needle_position, total_events
        ),
        task: format!("What is the {} we configured earlier?", key_name),
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
    let mut events = Vec::new();
    let total_events = 200;

    let service_name = "UserAuthService";
    let auth_method = "OAuth 2.0";
    let storage = "Redis";
    let port = "8443";

    let plants = [
        (
            10,
            format!("Let's call the authentication service {}", service_name),
        ),
        (
            45,
            format!(
                "{} will use {} for authentication flow",
                service_name, auth_method
            ),
        ),
        (
            80,
            format!("We'll cache {} sessions in {}", service_name, storage),
        ),
        (120, format!("{} should run on port {}", service_name, port)),
    ];

    for i in 0..total_events {
        let plant = plants.iter().find(|(pos, _)| *pos == i);

        if let Some((_, content)) = plant {
            events.push(Event::user(content.clone()));
        } else if i % 2 == 0 {
            events.push(Event::user(generate_filler_user_message(i)));
        } else {
            events.push(Event::assistant(generate_filler_assistant_message(i)));
        }
    }

    DatasetRecord {
        id: format!("multi_hop_{}", variant),
        description: "Synthesize service configuration from multiple points in conversation"
            .to_string(),
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
    let mut events = Vec::new();

    let function_versions = [
        (0, "fn calculate(x: i32) -> i32 { x }"),
        (20, "fn calculate(x: i32) -> i32 { x * 2 }"),
        (40, "fn calculate(x: i32, y: i32) -> i32 { (x + y) * 2 }"),
        (
            60,
            "fn calculate(x: i32, y: i32) -> i32 { ((x + y) * 2).max(0) }",
        ),
        (
            80,
            "fn calculate(x: i32, y: i32, factor: i32) -> i32 { ((x + y) * factor).max(0) }",
        ),
    ];

    let total_events = 150;

    for i in 0..total_events {
        let version = function_versions.iter().find(|(pos, _)| *pos == i);

        if let Some((_, code)) = version {
            events.push(Event::user(format!(
                "Update the function to: ```rust\n{}\n```",
                code
            )));
            events.push(Event::assistant(
                "Done, I've updated the calculate function.".to_string(),
            ));
        } else if i % 3 == 0 {
            events.push(Event::user(generate_filler_user_message(i)));
        } else if i % 3 == 1 {
            events.push(Event::assistant(generate_filler_assistant_message(i)));
        } else {
            events.push(Event::tool_result(
                "read_file",
                format!("call_{}", i),
                generate_filler_file_content(i),
            ));
        }
    }

    let final_version = function_versions.last().unwrap().1;

    DatasetRecord {
        id: format!("cumulative_{}", variant),
        description: "Track cumulative changes to a function across conversation".to_string(),
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
    let total_events = 180 + (variant * 20) % 60;
    let mut events = Vec::new();

    let positions: Vec<usize> = (0..choices.len())
        .map(|i| 5 + i * (total_events / choices.len()))
        .collect();

    let decisions: Vec<(usize, &str, &str)> = positions
        .iter()
        .zip(choices.iter())
        .map(|(pos, (name, stmt))| (*pos, *name, *stmt))
        .collect();

    for i in 0..total_events {
        let decision = decisions.iter().find(|(pos, _, _)| *pos == i);

        if let Some((_, _name, statement)) = decision {
            events.push(Event::user(statement.to_string()));
            events.push(Event::assistant(
                "Understood, I'll update the configuration.".to_string(),
            ));
        } else if i % 2 == 0 {
            events.push(Event::user(generate_filler_user_message(i + variant)));
        } else {
            events.push(Event::assistant(generate_filler_assistant_message(
                i + variant,
            )));
        }
    }

    let final_choice = decisions.last().unwrap().1;

    DatasetRecord {
        id: format!("final_decision_{}_{}", topic, variant),
        description: format!("Decision about {} changes {} times", topic, choices.len()),
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
    let total_events = 160 + (variant * 15) % 40;
    let mut events = Vec::new();

    let positions: Vec<usize> = (0..versions.len())
        .map(|i| 8 + i * (total_events / versions.len()))
        .collect();

    let version_list: Vec<(usize, &str, &str)> = positions
        .iter()
        .zip(versions.iter())
        .map(|(pos, (ver, stmt))| (*pos, *ver, *stmt))
        .collect();

    for i in 0..total_events {
        let version = version_list.iter().find(|(pos, _, _)| *pos == i);

        if let Some((_, ver, statement)) = version {
            events.push(Event::user(format!("{} - {}", statement, ver)));
            events.push(Event::assistant(format!("Updated to {}.", ver)));
        } else if i % 2 == 0 {
            events.push(Event::user(generate_filler_user_message(i + variant)));
        } else {
            events.push(Event::assistant(generate_filler_assistant_message(
                i + variant,
            )));
        }
    }

    let version_names: Vec<String> = versions.iter().map(|(v, _)| v.to_string()).collect();

    DatasetRecord {
        id: format!("timeline_{}_{}", topic, variant),
        description: format!("Recreate timeline of {} changes", topic),
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

    let (name, task, expected, tool_calls) = &tool_scenarios[variant % tool_scenarios.len()];
    let total_events = 140 + (variant * 20) % 40;
    let mut events = Vec::new();

    let positions: Vec<usize> = (0..tool_calls.len())
        .map(|i| 10 + i * (total_events / tool_calls.len()))
        .collect();

    let calls: Vec<(usize, &str, &str)> = positions
        .iter()
        .zip(tool_calls.iter())
        .map(|(pos, (file, content))| (*pos, *file, *content))
        .collect();

    for i in 0..total_events {
        let tool_call = calls.iter().find(|(pos, _, _)| *pos == i);

        if let Some((_, filename, content)) = tool_call {
            events.push(Event::user(format!("Read the file {}", filename)));
            events.push(Event::tool_result(
                "read_file",
                format!("read_file_{}", i),
                format!("Contents of {}:\n{}", filename, content),
            ));
            events.push(Event::assistant(format!("I've read {}.", filename)));
        } else if i % 2 == 0 {
            events.push(Event::user(generate_filler_user_message(i + variant)));
        } else {
            events.push(Event::assistant(generate_filler_assistant_message(
                i + variant,
            )));
        }
    }

    DatasetRecord {
        id: format!("tool_disambig_{}_{}", name, variant),
        description: "Same file read multiple times, must use most recent".to_string(),
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

/// Generate filler user messages
fn generate_filler_user_message(seed: usize) -> String {
    let templates = [
        "Can you help me refactor the logging module?",
        "What do you think about using async/await here?",
        "Let's add some error handling to this function",
        "Should we split this into separate files?",
        "I'm getting a type error on line 42",
        "Can you explain how this iterator works?",
        "Let's add tests for the edge cases",
        "What's the best way to handle this edge case?",
        "Should we use a hashmap or a vec here?",
        "Can you review this implementation?",
        "I think we need to add caching here",
        "How should we handle the timeout?",
        "Let's document this module",
        "Can you help debug this issue?",
        "What about thread safety here?",
    ];
    templates[seed % templates.len()].to_string()
}

/// Generate filler assistant messages
fn generate_filler_assistant_message(seed: usize) -> String {
    let templates = [
        "That's a good approach. Let me implement that.",
        "I'll refactor this to be more maintainable.",
        "Here's an updated version with better error handling.",
        "Good idea, I'll split this into separate concerns.",
        "The error is due to a type mismatch. Let me fix it.",
        "This iterator uses lazy evaluation to process items.",
        "I've added comprehensive test coverage.",
        "For this edge case, we should return early.",
        "A hashmap would be O(1) lookup, which is better here.",
        "The implementation looks good, just one suggestion.",
        "I'll add a simple LRU cache for this.",
        "We can use tokio's timeout wrapper for this.",
        "I've added documentation with examples.",
        "Found the bug - it's a off-by-one error.",
        "I'll add a mutex to protect the shared state.",
    ];
    templates[seed % templates.len()].to_string()
}

/// Generate filler file content for tool results
fn generate_filler_file_content(seed: usize) -> String {
    let templates = [
        "use std::collections::HashMap;\n\npub struct Config {\n    values: HashMap<String, String>,\n}",
        "mod tests {\n    use super::*;\n\n    #[test]\n    fn test_basic() {\n        assert!(true);\n    }\n}",
        "pub fn process(input: &str) -> Result<String, Error> {\n    Ok(input.to_uppercase())\n}",
        "/// Module documentation\n/// \n/// This module handles...",
        "const MAX_RETRIES: u32 = 3;\nconst TIMEOUT_MS: u64 = 5000;",
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
        assert!(
            types
                .iter()
                .any(|t| t.as_str() == "tool_result_disambiguation")
        );
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
