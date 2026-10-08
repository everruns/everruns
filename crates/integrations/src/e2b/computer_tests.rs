use std::collections::HashMap;
use std::sync::Mutex;

use super::*;
use base64::Engine;
use everruns_contracts::runtime::capabilities::Capability;
use everruns_contracts::runtime::session_services::SessionStorageStore;
use everruns_contracts::runtime::session_services::{KeyInfo, SecretInfo};
use everruns_contracts::typed_id::SessionId;

use everruns_contracts::error::Result as StoreResult;

fn action(value: Value) -> ComputerAction {
    ComputerAction::from_arguments(&value).unwrap()
}

fn commands(value: Value) -> Vec<Vec<String>> {
    xdotool_commands(&action(value)).unwrap()
}

fn strs(commands: &[Vec<String>]) -> Vec<Vec<&str>> {
    commands
        .iter()
        .map(|command| command.iter().map(String::as_str).collect())
        .collect()
}

// ---------------------------------------------------------------------------
// Action -> xdotool argv
// ---------------------------------------------------------------------------

#[test]
fn clicks_move_then_press_the_right_button() {
    let left = commands(json!({"action": "left_click", "coordinate": [10, 20]}));
    assert_eq!(
        strs(&left),
        vec![vec!["mousemove", "--sync", "10", "20"], vec!["click", "1"]]
    );

    let right = commands(json!({"action": "right_click", "coordinate": [5, 6]}));
    assert_eq!(strs(&right)[1], vec!["click", "3"]);

    let middle = commands(json!({"action": "middle_click"}));
    // No coordinate: click where the pointer already is.
    assert_eq!(strs(&middle), vec![vec!["click", "2"]]);

    let double = commands(json!({"action": "double_click", "coordinate": [1, 1]}));
    assert_eq!(strs(&double)[1], vec!["click", "--repeat", "2", "1"]);

    let triple = commands(json!({"action": "triple_click"}));
    assert_eq!(strs(&triple), vec![vec!["click", "--repeat", "3", "1"]]);
}

#[test]
fn modifiers_are_held_around_a_click_and_released_in_reverse() {
    let click = commands(json!({
        "action": "left_click", "coordinate": [3, 4], "text": "ctrl+shift"
    }));
    assert_eq!(
        strs(&click),
        vec![
            vec!["mousemove", "--sync", "3", "4"],
            vec!["keydown", "ctrl"],
            vec!["keydown", "shift"],
            vec!["click", "1"],
            vec!["keyup", "shift"],
            vec!["keyup", "ctrl"],
        ]
    );
}

#[test]
fn drag_presses_moves_through_a_midpoint_and_releases() {
    let drag = commands(json!({
        "action": "left_click_drag", "start_coordinate": [10, 10], "coordinate": [30, 50]
    }));
    assert_eq!(
        strs(&drag),
        vec![
            vec!["mousemove", "--sync", "10", "10"],
            vec!["mousedown", "1"],
            vec!["mousemove", "--sync", "20", "30"],
            vec!["mousemove", "--sync", "30", "50"],
            vec!["mouseup", "1"],
        ]
    );
}

#[test]
fn move_scroll_and_wait() {
    assert_eq!(
        strs(&commands(
            json!({"action": "mouse_move", "coordinate": [7, 8]})
        )),
        vec![vec!["mousemove", "--sync", "7", "8"]]
    );
    let cases = [("up", "4"), ("down", "5"), ("left", "6"), ("right", "7")];
    for (direction, button) in cases {
        let scroll = commands(json!({
            "action": "scroll", "coordinate": [1, 2],
            "scroll_direction": direction, "scroll_amount": 3
        }));
        assert_eq!(
            strs(&scroll),
            vec![
                vec!["mousemove", "--sync", "1", "2"],
                vec!["click", "--repeat", "3", button],
            ]
        );
    }
    assert!(commands(json!({"action": "wait", "duration": 1})).is_empty());
    assert!(commands(json!({"action": "screenshot"})).is_empty());
}

#[test]
fn typed_text_is_one_argv_entry_after_the_option_terminator() {
    // Shell metacharacters, quotes, a leading dash, a newline and Unicode:
    // with no shell in the path, every byte must reach xdotool untouched.
    let hostile = "-rf $(rm -rf /) `id` ; echo \"pwned\" | tee 'x' && $HOME > /dev/null\nпривіт *";
    let typed = commands(json!({"action": "type", "text": hostile}));
    assert_eq!(typed.len(), 1);
    assert_eq!(typed[0][..4], ["type", "--delay", "12", "--"]);
    assert_eq!(typed[0][4], hostile);
    assert_eq!(typed[0].len(), 5);
}

#[test]
fn keys_map_to_x_keysyms() {
    let key = |text: &str| xdotool_key_combo(text).unwrap();
    assert_eq!(key("Return"), "Return");
    assert_eq!(key("Enter"), "Return");
    assert_eq!(key("ctrl+a"), "ctrl+a");
    assert_eq!(key("cmd+shift+T"), "super+shift+T");
    assert_eq!(key("PageDown"), "Page_Down");
    assert_eq!(key("Backspace"), "BackSpace");
    assert_eq!(key("ArrowUp"), "Up");
    assert_eq!(key("F12"), "F12");
    assert_eq!(key("+"), "plus");
    assert_eq!(key("ctrl+-"), "ctrl+minus");
    assert_eq!(key("XF86AudioMute"), "XF86AudioMute");
    assert!(xdotool_key_combo("ctrl+$(id)").is_err());
    assert!(xdotool_key_combo("hyper+a").is_err());
}

#[test]
fn key_repeat_sends_the_combo_as_separate_keystrokes() {
    let keys = commands(json!({"action": "key", "text": "Tab", "repeat": 3}));
    assert_eq!(strs(&keys), vec![vec!["key", "--", "Tab", "Tab", "Tab"]]);
}

#[test]
fn navigate_is_refused_on_a_desktop() {
    let err =
        xdotool_commands(&action(json!({"action": "navigate", "url": "https://a.b"}))).unwrap_err();
    assert!(err.contains("desktop"), "{err}");
}

#[test]
fn the_display_script_gets_values_as_positional_arguments() {
    let args = start_display_args(DisplaySize {
        width: 1024,
        height: 768,
    });
    assert_eq!(args[0], "-c");
    assert_eq!(args[1], START_DISPLAY_SCRIPT);
    assert_eq!(args[3..], [":99", "1024x768", WORK_DIR]);
}

// ---------------------------------------------------------------------------
// Screenshot decoding
// ---------------------------------------------------------------------------

/// A minimal PNG header: signature plus an IHDR chunk.
fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend(13u32.to_be_bytes());
    bytes.extend(b"IHDR");
    bytes.extend(width.to_be_bytes());
    bytes.extend(height.to_be_bytes());
    bytes.extend([8, 6, 0, 0, 0]);
    bytes.extend([0, 0, 0, 0]);
    bytes
}

#[test]
fn screenshots_decode_to_base64_png_at_the_display_size() {
    let display = DisplaySize {
        width: 1280,
        height: 800,
    };
    let bytes = png(1280, 800);
    assert_eq!(png_dimensions(&bytes).unwrap(), (1280, 800));
    let shot = decode_screenshot(&bytes, display).unwrap();
    assert_eq!(shot.media_type, "image/png");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&shot.base64)
            .unwrap(),
        bytes
    );
}

#[test]
fn screenshots_of_the_wrong_size_or_format_are_errors() {
    let display = DisplaySize {
        width: 1280,
        height: 800,
    };
    let err = decode_screenshot(&png(1024, 768), display).unwrap_err();
    assert!(err.contains("1024x768"), "{err}");
    assert!(decode_screenshot(b"\xFF\xD8\xFF\xE0 jpeg", display).is_err());
    assert!(decode_screenshot(&[], display).is_err());
}

// ---------------------------------------------------------------------------
// Capability: the shared tool, its validation and its budget
// ---------------------------------------------------------------------------

#[derive(Default)]
struct MemoryStore(Mutex<HashMap<String, String>>);

#[async_trait]
impl SessionStorageStore for MemoryStore {
    async fn set_value(&self, _: SessionId, key: &str, value: &str) -> StoreResult<()> {
        self.0
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }
    async fn get_value(&self, _: SessionId, key: &str) -> StoreResult<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    async fn delete_value(&self, _: SessionId, key: &str) -> StoreResult<bool> {
        Ok(self.0.lock().unwrap().remove(key).is_some())
    }
    async fn list_keys(&self, _: SessionId) -> StoreResult<Vec<KeyInfo>> {
        Ok(vec![])
    }
    async fn set_secret(&self, _: SessionId, _: &str, _: &str) -> StoreResult<()> {
        Ok(())
    }
    async fn get_secret(&self, _: SessionId, _: &str) -> StoreResult<Option<String>> {
        Ok(None)
    }
    async fn delete_secret(&self, _: SessionId, _: &str) -> StoreResult<bool> {
        Ok(false)
    }
    async fn list_secrets(&self, _: SessionId) -> StoreResult<Vec<SecretInfo>> {
        Ok(vec![])
    }
}

#[test]
fn capability_exposes_one_desktop_computer_tool() {
    let cap = E2BDesktopComputerUseCapability;
    assert_eq!(cap.id(), DESKTOP_COMPUTER_USE_CAPABILITY_ID);
    let tools = cap.tools_with_config(&json!({"display_width": 1024, "display_height": 768}));
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name(), "computer");
    assert!(tools[0].description().contains("1024x768"));
    // Not a browser: the schema offers no navigate action.
    assert!(
        !tools[0]
            .parameters_schema()
            .to_string()
            .contains("navigate")
    );
    assert!(
        cap.validate_config(&json!({"display_width": 99999}))
            .is_err()
    );
    assert!(
        cap.system_prompt_addition()
            .unwrap()
            .contains("untrusted data")
    );
    assert!(cap.pre_tool_use_hooks().is_empty());
    assert_eq!(cap.risk_level(), RiskLevel::High);
}

#[tokio::test]
async fn shared_validation_and_budget_run_before_the_sandbox_is_touched() {
    let cap = E2BDesktopComputerUseCapability;
    let tool = cap
        .tools_with_config(&json!({"max_actions_per_session": 1}))
        .remove(0);
    let storage = Arc::new(MemoryStore::default());
    let context = ToolContext::new(SessionId::new()).with_storage_store_arc(storage.clone());

    // Out of bounds: rejected by the shared validator, not charged.
    let result = tool
        .execute_with_context(
            json!({"action": "left_click", "coordinate": [5000, 5]}),
            &context,
        )
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => assert!(msg.contains("outside"), "{msg}"),
        other => panic!("expected a validation error, got {other:?}"),
    }

    // Valid: charged, then the backend asks for an E2B connection.
    let result = tool
        .execute_with_context(json!({"action": "screenshot"}), &context)
        .await;
    assert!(
        format!("{result:?}").to_lowercase().contains("e2b"),
        "expected an E2B connection request, got {result:?}"
    );

    // The budget of one is spent.
    let result = tool
        .execute_with_context(json!({"action": "screenshot"}), &context)
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => assert!(msg.contains("budget"), "{msg}"),
        other => panic!("expected the budget error, got {other:?}"),
    }
}

#[test]
fn the_desktop_sandbox_key_is_reserved_from_kv_store() {
    assert!(
        everruns_core::host::session_services::capabilities::is_internal_session_kv_key(
            &desktop_sandbox_key()
        )
    );
}
