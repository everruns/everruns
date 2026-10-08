use super::*;

fn call(value: Value) -> Result<BrowserCall, String> {
    BrowserCall::from_arguments(&value)
}

fn viewport() -> DisplaySize {
    DisplaySize {
        width: 800,
        height: 600,
    }
}

#[test]
fn parses_the_toolset_shaped_vocabulary() {
    let click = call(json!({
        "action": "left_click", "target": {"type": "ref", "ref": "ref_3"},
        "modifiers": "shift", "tab_id": "T1"
    }))
    .unwrap();
    assert_eq!(click.tab_id.as_deref(), Some("T1"));
    assert_eq!(
        click.action,
        BrowserAction::LeftClick {
            target: Target::Ref {
                reference: "ref_3".into()
            },
            modifiers: Some("shift".into()),
        }
    );
    let drag = call(json!({
        "action": "left_click_drag",
        "from": {"type": "coordinate", "x": 1, "y": 2},
        "target": {"type": "coordinate", "x": 3, "y": 4}
    }))
    .unwrap();
    assert_eq!(drag.action.name(), "left_click_drag");
    let read =
        call(json!({"action": "read_page", "filter": "interactive", "depth": 3, "ref": "ref_1"}))
            .unwrap();
    assert_eq!(
        read.action,
        BrowserAction::ReadPage {
            filter: Some(ReadFilter::Interactive),
            depth: Some(3),
            reference: Some("ref_1".into()),
        }
    );
    for bare in ["screenshot", "get_page_text", "new_tab", "list_tabs"] {
        assert_eq!(call(json!({"action": bare})).unwrap().action.name(), bare);
    }
}

#[test]
fn members_shipped_off_are_refused_by_name() {
    for name in DISABLED_BROWSER_ACTIONS {
        let err = call(json!({"action": name, "text": "1+1"})).unwrap_err();
        assert_eq!(err, format!("{name} is not enabled in this environment."));
    }
    assert!(
        call(json!({"action": "teleport"}))
            .unwrap_err()
            .contains("Invalid browser action `teleport`")
    );
}

#[test]
fn validation_checks_targets_bounds_and_limits() {
    let invalid = [
        json!({"action": "left_click", "target": {"type": "coordinate", "x": 800, "y": 1}}),
        json!({"action": "left_click", "target": {"type": "ref", "ref": " "}}),
        json!({"action": "left_click", "target": {"type": "ref", "ref": "ref_1"}, "modifiers": "hyper"}),
        json!({"action": "mouse_move", "target": {"type": "ref", "ref": "ref_1"}}),
        json!({"action": "scroll_to", "target": {"type": "coordinate", "x": 1, "y": 1}}),
        json!({"action": "scroll", "target": {"type": "coordinate", "x": 1, "y": 1}, "scroll_direction": "down", "scroll_amount": 11}),
        json!({"action": "zoom", "region": [10, 10, 5, 20]}),
        json!({"action": "key", "text": "  "}),
        json!({"action": "key", "text": "Enter", "repeat": 0}),
        json!({"action": "wait", "duration": 31}),
        json!({"action": "hold_key", "text": "shift", "duration": -1}),
        json!({"action": "read_page", "depth": 0}),
        json!({"action": "find", "query": ""}),
        json!({"action": "form_input", "target": {"type": "ref", "ref": "ref_1"}, "value": [1]}),
        json!({"action": "switch_tab"}),
        json!({"action": "screenshot", "tab_id": ""}),
    ];
    for arguments in invalid {
        let parsed = call(arguments.clone()).unwrap();
        assert!(parsed.validate(viewport()).is_err(), "{arguments}");
    }
    let valid = [
        json!({"action": "scroll", "target": {"type": "coordinate", "x": 1, "y": 1}, "scroll_direction": "down"}),
        json!({"action": "key", "text": "Backspace Backspace ctrl+a", "repeat": 2}),
        json!({"action": "form_input", "target": {"type": "ref", "ref": "ref_1"}, "value": true}),
        json!({"action": "switch_tab", "tab_id": "T2"}),
        json!({"action": "zoom", "region": [0, 0, 800, 600]}),
    ];
    for arguments in valid {
        call(arguments.clone())
            .unwrap()
            .validate(viewport())
            .unwrap_or_else(|e| panic!("{arguments}: {e}"));
    }
}

#[test]
fn state_text_is_cleaned_and_capped() {
    assert_eq!(clean_state_text("  a\nb\u{2028}c\t "), "a b c");
    assert_eq!(
        clean_state_text(&"x".repeat(5000)).len(),
        MAX_STATE_TEXT_CHARS
    );
}

#[test]
fn config_defaults_and_bounds() {
    let config = BrowserUseConfig::from_value(&Value::Null).unwrap();
    assert_eq!(config.viewport(), DisplaySize::default());
    assert!(BrowserUseConfig::from_value(&json!({"viewport_width": 100})).is_err());
    assert!(BrowserUseConfig::from_value(&json!({"max_actions_per_session": 0})).is_err());
    assert!(BrowserUseConfig::from_value(&json!({"bogus": 1})).is_err());
}

#[test]
fn schema_lists_every_offered_member_and_none_shipped_off() {
    let schema = browser_tool_schema(viewport());
    let actions: Vec<&str> = schema["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(actions.len(), 27);
    for name in DISABLED_BROWSER_ACTIONS {
        assert!(!actions.contains(&name));
    }
}
