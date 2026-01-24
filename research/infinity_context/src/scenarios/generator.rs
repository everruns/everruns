//! Synthetic scenario generation
//!
//! Uses everruns-core Message type for test scenarios.

use crate::types::{message_helpers, ExpectedResult, PlantedInfo, Scenario, ScenarioType};

/// Generate synthetic test scenarios
pub fn generate_synthetic(count: usize) -> Vec<Scenario> {
    let mut scenarios = Vec::new();

    // Always include one of each type
    scenarios.push(generate_needle_scenario(0));
    scenarios.push(generate_multi_hop_scenario(0));
    scenarios.push(generate_cumulative_scenario(0));
    scenarios.push(generate_final_decision_scenario(0));
    scenarios.push(generate_decision_timeline_scenario(0));
    scenarios.push(generate_tool_disambiguation_scenario(0));

    // Add more needle scenarios (most common test case)
    for i in 1..count.saturating_sub(5) {
        scenarios.push(generate_needle_scenario(i));
    }

    scenarios
}

/// Generate a needle-in-haystack scenario
fn generate_needle_scenario(variant: usize) -> Scenario {
    // Fake secrets for evaluation scenarios - NOT real credentials
    let secrets = [
        ("API_KEY", "fake-api-key-abc123xyz789"),
        ("DATABASE_PASSWORD", "fake-db-password-42"),
        ("JWT_SECRET", "fake-jwt-secret-key"),
        ("ACCESS_KEY", "FAKE-ACCESS-KEY-EXAMPLE"),
        ("PAYMENT_KEY", "fake-payment-key-example"),
    ];

    let (key_name, key_value) = secrets[variant % secrets.len()];
    let needle_position = 15 + (variant * 7) % 30; // Vary where the needle is placed
    let total_messages = 150 + (variant * 20); // Vary conversation length

    let mut messages = Vec::new();

    // Generate conversation with planted needle
    for i in 0..total_messages {
        if i == needle_position {
            // Plant the needle
            messages.push(message_helpers::user(format!(
                "By the way, I set the {} to {} in the .env file",
                key_name, key_value
            )));
        } else if i % 2 == 0 {
            // User message (filler)
            messages.push(message_helpers::user(generate_filler_user_message(i)));
        } else {
            // Assistant message (filler)
            messages.push(message_helpers::assistant(generate_filler_assistant_message(i)));
        }
    }

    Scenario {
        name: format!("needle_basic_{}", variant),
        description: format!(
            "Find {} value planted at message {} in {} message conversation",
            key_name, needle_position, total_messages
        ),
        scenario_type: ScenarioType::NeedleInHaystack,
        messages,
        task: format!("What is the {} we configured earlier?", key_name),
        expected: ExpectedResult::Contains {
            contains: vec![key_value.to_string()],
        },
        planted_info: vec![PlantedInfo {
            message_index: needle_position,
            key: key_name.to_string(),
            value: key_value.to_string(),
        }],
    }
}

/// Generate a multi-hop scenario requiring synthesis from multiple points
fn generate_multi_hop_scenario(variant: usize) -> Scenario {
    let mut messages = Vec::new();
    let total_messages = 200;

    // Plant information at different points
    let service_name = "UserAuthService";
    let auth_method = "OAuth 2.0";
    let storage = "Redis";
    let port = "8443";

    let plants = [
        (10, format!("Let's call the authentication service {}", service_name)),
        (45, format!("{} will use {} for authentication flow", service_name, auth_method)),
        (80, format!("We'll cache {} sessions in {}", service_name, storage)),
        (120, format!("{} should run on port {}", service_name, port)),
    ];

    for i in 0..total_messages {
        // Check if this is a plant position
        let plant = plants.iter().find(|(pos, _)| *pos == i);

        if let Some((_, content)) = plant {
            messages.push(message_helpers::user(content.clone()));
        } else if i % 2 == 0 {
            messages.push(message_helpers::user(generate_filler_user_message(i)));
        } else {
            messages.push(message_helpers::assistant(generate_filler_assistant_message(i)));
        }
    }

    Scenario {
        name: format!("multi_hop_{}", variant),
        description: "Synthesize service configuration from multiple points in conversation".to_string(),
        scenario_type: ScenarioType::MultiHop,
        messages,
        task: format!(
            "Summarize the complete configuration for {}: what authentication method, caching, and port?",
            service_name
        ),
        expected: ExpectedResult::Contains {
            contains: vec![
                auth_method.to_string(),
                storage.to_string(),
                port.to_string(),
            ],
        },
        planted_info: plants
            .iter()
            .map(|(idx, content)| PlantedInfo {
                message_index: *idx,
                key: format!("info_{}", idx),
                value: content.clone(),
            })
            .collect(),
    }
}

/// Generate a cumulative scenario tracking changes over time
fn generate_cumulative_scenario(variant: usize) -> Scenario {
    let mut messages = Vec::new();

    // Build up a function incrementally
    let function_versions = [
        (0, "fn calculate(x: i32) -> i32 { x }"),
        (20, "fn calculate(x: i32) -> i32 { x * 2 }"),
        (40, "fn calculate(x: i32, y: i32) -> i32 { (x + y) * 2 }"),
        (60, "fn calculate(x: i32, y: i32) -> i32 { ((x + y) * 2).max(0) }"),
        (80, "fn calculate(x: i32, y: i32, factor: i32) -> i32 { ((x + y) * factor).max(0) }"),
    ];

    let total_messages = 150;

    for i in 0..total_messages {
        // Check if this is a version update
        let version = function_versions.iter().find(|(pos, _)| *pos == i);

        if let Some((_, code)) = version {
            messages.push(message_helpers::user(format!(
                "Update the function to: ```rust\n{}\n```",
                code
            )));
            messages.push(message_helpers::assistant(
                "Done, I've updated the calculate function.".to_string(),
            ));
        } else if i % 3 == 0 {
            messages.push(message_helpers::user(generate_filler_user_message(i)));
        } else if i % 3 == 1 {
            messages.push(message_helpers::assistant(generate_filler_assistant_message(i)));
        } else {
            // Add some tool results for variety - using a generic tool call id
            messages.push(message_helpers::tool_result(
                format!("call_{}", i),
                generate_filler_file_content(i),
            ));
        }
    }

    let _final_version = function_versions.last().unwrap().1;

    Scenario {
        name: format!("cumulative_{}", variant),
        description: "Track cumulative changes to a function across conversation".to_string(),
        scenario_type: ScenarioType::Cumulative,
        messages,
        task: "What is the current/final version of the calculate function? Show the complete function signature and body.".to_string(),
        expected: ExpectedResult::Contains {
            contains: vec![
                "factor".to_string(),
                "max(0)".to_string(),
            ],
        },
        planted_info: function_versions
            .iter()
            .map(|(idx, code)| PlantedInfo {
                message_index: *idx,
                key: format!("version_{}", idx),
                value: code.to_string(),
            })
            .collect(),
    }
}

/// Generate a scenario where a decision changes multiple times, task asks for final state
fn generate_final_decision_scenario(_variant: usize) -> Scenario {
    let mut messages = Vec::new();
    let total_messages = 180;

    // Database choice changes multiple times
    let decisions = [
        (5, "PostgreSQL", "Let's use PostgreSQL for the database"),
        (35, "MySQL", "Actually, let's switch to MySQL instead - better for our use case"),
        (70, "MongoDB", "On second thought, MongoDB would be better for the schema flexibility"),
        (110, "PostgreSQL", "After more research, let's go back to PostgreSQL with JSONB columns"),
        (150, "CockroachDB", "Final decision: we'll use CockroachDB for the distributed nature"),
    ];

    for i in 0..total_messages {
        let decision = decisions.iter().find(|(pos, _, _)| *pos == i);

        if let Some((_, _db, statement)) = decision {
            messages.push(message_helpers::user(statement.to_string()));
            messages.push(message_helpers::assistant(
                "Understood, I'll update the configuration for the new database choice.".to_string(),
            ));
        } else if i % 2 == 0 {
            messages.push(message_helpers::user(generate_filler_user_message(i)));
        } else {
            messages.push(message_helpers::assistant(generate_filler_assistant_message(i)));
        }
    }

    let final_db = decisions.last().unwrap().1;

    Scenario {
        name: "final_decision_0".to_string(),
        description: "Decision changes 5 times, must identify the final choice".to_string(),
        scenario_type: ScenarioType::FinalDecision,
        messages,
        task: "What database did we finally decide to use for this project?".to_string(),
        expected: ExpectedResult::Contains {
            contains: vec![final_db.to_string()],
        },
        planted_info: decisions
            .iter()
            .map(|(idx, db, statement)| PlantedInfo {
                message_index: *idx,
                key: format!("decision_{}", idx),
                value: format!("{}: {}", db, statement),
            })
            .collect(),
    }
}

/// Generate a scenario where task asks to recreate the timeline of decision changes
fn generate_decision_timeline_scenario(_variant: usize) -> Scenario {
    let mut messages = Vec::new();
    let total_messages = 160;

    // API version changes over time
    let versions = [
        (8, "v1", "Let's start with API version v1"),
        (40, "v2", "We need to bump to v2 for the breaking changes"),
        (75, "v2.1", "Minor update to v2.1 for the bugfixes"),
        (120, "v3", "Major release: moving to v3 with the new architecture"),
    ];

    for i in 0..total_messages {
        let version = versions.iter().find(|(pos, _, _)| *pos == i);

        if let Some((_, ver, statement)) = version {
            messages.push(message_helpers::user(format!("{} - {}", statement, ver)));
            messages.push(message_helpers::assistant(format!(
                "I'll update the API version to {}.",
                ver
            )));
        } else if i % 2 == 0 {
            messages.push(message_helpers::user(generate_filler_user_message(i)));
        } else {
            messages.push(message_helpers::assistant(generate_filler_assistant_message(i)));
        }
    }

    Scenario {
        name: "decision_timeline_0".to_string(),
        description: "Must recreate the timeline of API version changes".to_string(),
        scenario_type: ScenarioType::DecisionTimeline,
        messages,
        task: "List all the API versions we went through in order, from first to last.".to_string(),
        expected: ExpectedResult::Contains {
            contains: vec![
                "v1".to_string(),
                "v2".to_string(),
                "v2.1".to_string(),
                "v3".to_string(),
            ],
        },
        planted_info: versions
            .iter()
            .map(|(idx, ver, statement)| PlantedInfo {
                message_index: *idx,
                key: format!("version_{}", idx),
                value: format!("{}: {}", ver, statement),
            })
            .collect(),
    }
}

/// Generate a scenario with same tool called multiple times, results must not be mixed
fn generate_tool_disambiguation_scenario(_variant: usize) -> Scenario {
    let mut messages = Vec::new();
    let total_messages = 140;

    // File read tool called for different files at different times
    let tool_calls = [
        (10, "config.json", r#"{"port": 3000, "host": "localhost"}"#),
        (45, "users.json", r#"[{"id": 1, "name": "Alice"}, {"id": 2, "name": "Bob"}]"#),
        (80, "config.json", r#"{"port": 8080, "host": "0.0.0.0", "ssl": true}"#),
        (115, "settings.yaml", "debug: true\nlog_level: info"),
    ];

    for i in 0..total_messages {
        let tool_call = tool_calls.iter().find(|(pos, _, _)| *pos == i);

        if let Some((_, filename, content)) = tool_call {
            messages.push(message_helpers::user(format!("Read the file {}", filename)));
            messages.push(message_helpers::tool_result(
                format!("read_file_{}", i),
                format!("Contents of {}:\n{}", filename, content),
            ));
            messages.push(message_helpers::assistant(format!(
                "I've read {}. Let me know if you need anything else.",
                filename
            )));
        } else if i % 2 == 0 {
            messages.push(message_helpers::user(generate_filler_user_message(i)));
        } else {
            messages.push(message_helpers::assistant(generate_filler_assistant_message(i)));
        }
    }

    // Task asks about the SECOND read of config.json (at position 80)
    Scenario {
        name: "tool_disambiguation_0".to_string(),
        description: "Same file read twice with different content, must identify correct version".to_string(),
        scenario_type: ScenarioType::ToolResultDisambiguation,
        messages,
        task: "What is the current port number in config.json? (Use the most recent read)".to_string(),
        expected: ExpectedResult::Contains {
            contains: vec!["8080".to_string()],
        },
        planted_info: tool_calls
            .iter()
            .map(|(idx, filename, content)| PlantedInfo {
                message_index: *idx,
                key: format!("{}_{}", filename, idx),
                value: content.to_string(),
            })
            .collect(),
    }
}

/// Generate filler user messages about coding
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
    use crate::types::MessageExt;

    #[test]
    fn test_generate_synthetic_scenarios() {
        let scenarios = generate_synthetic(5);

        assert!(scenarios.len() >= 3);

        // Should have different types
        let types: Vec<_> = scenarios.iter().map(|s| &s.scenario_type).collect();
        assert!(types.iter().any(|t| matches!(t, ScenarioType::NeedleInHaystack)));
        assert!(types.iter().any(|t| matches!(t, ScenarioType::MultiHop)));
        assert!(types.iter().any(|t| matches!(t, ScenarioType::Cumulative)));
    }

    #[test]
    fn test_needle_scenario_has_planted_info() {
        let scenario = generate_needle_scenario(0);

        assert!(!scenario.planted_info.is_empty());
        assert!(!scenario.messages.is_empty());

        // Verify the planted message exists at the specified index
        let plant = &scenario.planted_info[0];
        let planted_message = &scenario.messages[plant.message_index];
        assert!(planted_message.text_content().contains(&plant.value));
    }
}
