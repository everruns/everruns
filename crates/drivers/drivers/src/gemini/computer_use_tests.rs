use super::*;
use everruns_contracts::tool_types::{BuiltinTool, ToolDefinition};

fn computer_tool(navigation: bool) -> ToolDefinition {
    let mut actions = vec!["screenshot", "left_click"];
    if navigation {
        actions.push("navigate");
    }
    ToolDefinition::Builtin(BuiltinTool {
        name: "computer".to_string(),
        display_name: None,
        description: String::new(),
        parameters: json!({ "properties": { "action": { "enum": actions } } }),
        policy: Default::default(),
        category: None,
        deferrable: Default::default(),
        hints: Default::default(),
        full_parameters: None,
    })
}

fn config(model: &str, navigation: bool) -> LlmCallConfig {
    let mut config = LlmCallConfig::new(model);
    config.tools = vec![computer_tool(navigation)];
    let (key, value) = NativeComputerUse {
        display_width: 1280,
        display_height: 800,
    }
    .to_driver_option();
    config.driver_options.insert(key, value);
    config
}

fn convert(name: &str, arguments: Value, batch: &mut Option<String>) -> ToolCall {
    let computer = GeminiComputer::for_call(&config("gemini-4-hydrogen", true)).unwrap();
    let mut call = ToolCall {
        id: format!("id_{name}"),
        name: name.into(),
        arguments,
    };
    computer.into_computer_call(&mut call, batch);
    call
}

/// Neutral action names the `computer` tool takes.
const NEUTRAL: [&str; 18] = [
    "screenshot",
    "left_click",
    "right_click",
    "middle_click",
    "double_click",
    "triple_click",
    "left_click_drag",
    "left_mouse_down",
    "left_mouse_up",
    "cursor_position",
    "zoom",
    "hold_key",
    "mouse_move",
    "scroll",
    "type",
    "key",
    "wait",
    "navigate",
];

fn assert_neutral(arguments: &Value, name: &str) {
    let actions = match arguments.get("actions") {
        Some(actions) => actions.as_array().unwrap().clone(),
        None => vec![arguments.clone()],
    };
    for action in actions {
        let action = action["action"].as_str().unwrap_or_default().to_string();
        assert!(NEUTRAL.contains(&action.as_str()), "{name}: {action}");
    }
}

#[test]
fn needs_a_gemini_4_model_the_option_and_the_computer_tool() {
    assert!(GeminiComputer::for_call(&config("gemini-3.8-flash", false)).is_none());
    let mut plain = config("gemini-4-hydrogen", false);
    plain.driver_options.clear();
    assert!(GeminiComputer::for_call(&plain).is_none());

    let desktop = GeminiComputer::for_call(&config("gemini-4-hydrogen", false)).unwrap();
    assert_eq!(
        desktop.tool_entry(),
        json!({ "type": "computer_use", "environment": "desktop",
                "excluded_predefined_functions": ["key_down", "key_up"] })
    );
    let browser = GeminiComputer::for_call(&config("gemini-4-neon", true)).unwrap();
    assert_eq!(browser.tool_entry()["environment"], "browser");
}

#[test]
fn actions_become_computer_calls_on_real_pixels() {
    let mut batch = None;
    let click = convert(
        "click",
        json!({ "x": 500, "y": 999, "intent": "Open the menu" }),
        &mut batch,
    );
    assert_eq!(click.name, "computer");
    assert_eq!(click.arguments["action"], "left_click");
    // 500/1000 of 1280 and the last row of 800.
    assert_eq!(click.arguments["coordinate"], json!([640, 799]));
    assert_eq!(click.arguments["native_batch"]["id"], "id_click");
    assert_eq!(click.arguments["native_batch"]["call"]["name"], "click");
    assert!(click.arguments.get("intent").is_none());

    // A later call of the same response joins its batch.
    let typed = convert(
        "type",
        json!({ "text": "Ada", "press_enter": true, "intent": "Search" }),
        &mut batch,
    );
    assert_eq!(
        typed.arguments["actions"],
        json!([{ "action": "type", "text": "Ada" }, { "action": "key", "text": "Return" }])
    );
    assert_eq!(typed.arguments["native_batch"]["id"], "id_click");
}

#[test]
fn every_action_maps_to_valid_neutral_arguments() {
    let cases = [
        ("double_click", json!({ "x": 1, "y": 2 })),
        ("triple_click", json!({ "x": 1, "y": 2 })),
        ("middle_click", json!({ "x": 1, "y": 2 })),
        ("right_click", json!({ "x": 1, "y": 2 })),
        ("mouse_down", json!({ "x": 1, "y": 2 })),
        ("mouse_up", json!({ "x": 1, "y": 2 })),
        ("move", json!({ "x": 1, "y": 2 })),
        (
            "drag_and_drop",
            json!({ "start_x": 0, "start_y": 0, "end_x": 900, "end_y": 900 }),
        ),
        ("wait", json!({})),
        ("press_key", json!({ "key": "Enter" })),
        ("hotkey", json!({ "keys": ["Control", "c"] })),
        ("take_screenshot", json!({})),
        (
            "scroll",
            json!({ "x": 500, "y": 500, "direction": "down", "magnitude_in_pixels": 800 }),
        ),
        ("go_back", json!({})),
        ("go_forward", json!({})),
        ("navigate", json!({ "url": "https://example.com" })),
    ];
    for (name, arguments) in cases {
        assert!(GeminiComputer::is_action(name), "{name}");
        let call = convert(name, arguments, &mut None);
        assert_neutral(&call.arguments, name);
    }
    let hotkey = convert(
        "hotkey",
        json!({ "keys": ["Control", "Shift", "t"] }),
        &mut None,
    );
    assert_eq!(hotkey.arguments["text"], "ctrl+shift+t");
    let scroll = convert(
        "scroll",
        json!({ "x": 0, "y": 0, "direction": "DOWN", "magnitude_in_pixels": 800 }),
        &mut None,
    );
    assert_eq!(scroll.arguments["scroll_amount"], 8);
    assert_eq!(scroll.arguments["scroll_direction"], "down");
    assert!(!GeminiComputer::is_action("key_down"));
}

#[test]
fn a_broken_action_reaches_the_tool_as_an_error() {
    let call = convert("click", json!({ "intent": "no point" }), &mut None);
    assert_eq!(call.arguments["action"], "click: missing `x`");
}

#[test]
fn safety_decisions_ride_along_and_are_acknowledged_once_run() {
    let decision = json!({ "decision": "require_confirmation", "explanation": "Buys a thing" });
    let call = convert(
        "click",
        json!({ "x": 1, "y": 1, "safety_decision": decision }),
        &mut None,
    );
    assert_eq!(call.arguments["pending_safety_checks"], json!([decision]));

    let ran = result_items(
        vec![
            json!({ "type": "text", "text": "{\"status\":\"ok\"}" }),
            json!({ "type": "image", "data": "AAA", "mime_type": "image/png" }),
        ],
        &call.arguments,
    );
    let body: Value = serde_json::from_str(ran[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        body,
        json!({ "status": "ok", "safety_acknowledgement": true })
    );
    assert_eq!(ran[1]["type"], "image");

    let refused = result_items(
        vec![json!({ "type": "text", "text": "Denied by the person" })],
        &call.arguments,
    );
    let body: Value = serde_json::from_str(refused[0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body, json!({ "output": "Denied by the person" }));
}

#[test]
fn replay_finds_the_original_call() {
    let call = convert(
        "navigate",
        json!({ "url": "https://a.test", "intent": "go" }),
        &mut None,
    );
    let (name, arguments) = original_call(&call.arguments).unwrap();
    assert_eq!(name, "navigate");
    assert_eq!(arguments["url"], "https://a.test");
    assert!(original_call(&json!({ "action": "screenshot" })).is_none());
}
