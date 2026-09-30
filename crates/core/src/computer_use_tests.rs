use super::*;
use crate::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
use everruns_provider::error::Result as StoreResult;
use everruns_provider::typed_id::SessionId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Doubles
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Recorder {
    performed: Mutex<Vec<ComputerAction>>,
    screenshots: Mutex<u32>,
    releases: Mutex<u32>,
    acquired_at: Mutex<Vec<DisplaySize>>,
}

struct FakeBackend {
    recorder: Arc<Recorder>,
    navigation: bool,
    fail_perform: bool,
}

struct FakeSession {
    recorder: Arc<Recorder>,
    display: DisplaySize,
    fail_perform: bool,
}

#[async_trait]
impl ComputerSession for FakeSession {
    fn display(&self) -> DisplaySize {
        self.display
    }

    async fn perform(&mut self, action: &ComputerAction) -> Result<(), String> {
        if self.fail_perform {
            return Err("element moved".to_string());
        }
        self.recorder.performed.lock().unwrap().push(action.clone());
        Ok(())
    }

    async fn screenshot(&mut self) -> Result<Screenshot, String> {
        *self.recorder.screenshots.lock().unwrap() += 1;
        Ok(Screenshot {
            base64: "iVBORw0KGgo=".to_string(),
            media_type: "image/png".to_string(),
        })
    }

    async fn release(self: Box<Self>) {
        *self.recorder.releases.lock().unwrap() += 1;
    }
}

#[async_trait]
impl ComputerBackend for FakeBackend {
    fn id(&self) -> &str {
        "fake"
    }

    fn supports_navigation(&self) -> bool {
        self.navigation
    }

    async fn acquire(
        &self,
        _context: &ToolContext,
        display: DisplaySize,
    ) -> Result<Box<dyn ComputerSession>, ToolExecutionResult> {
        self.recorder.acquired_at.lock().unwrap().push(display);
        Ok(Box::new(FakeSession {
            recorder: self.recorder.clone(),
            display,
            fail_perform: self.fail_perform,
        }))
    }
}

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

fn tool_with(config: ComputerUseConfig, navigation: bool) -> (ComputerTool, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let backend = Arc::new(FakeBackend {
        recorder: recorder.clone(),
        navigation,
        fail_perform: false,
    });
    (ComputerTool::new(backend, config), recorder)
}

fn context() -> ToolContext {
    ToolContext::new(SessionId::new()).with_storage_store_arc(Arc::new(MemoryStore::default()))
}

// ---------------------------------------------------------------------------
// Action parsing
// ---------------------------------------------------------------------------

#[test]
fn parses_the_anthropic_shaped_vocabulary() {
    let cases = [
        (json!({"action": "screenshot"}), ComputerAction::Screenshot),
        (
            json!({"action": "left_click", "coordinate": [10, 20]}),
            ComputerAction::LeftClick {
                coordinate: Some([10, 20]),
                text: None,
            },
        ),
        (
            json!({"action": "left_click_drag", "start_coordinate": [1, 2], "coordinate": [3, 4]}),
            ComputerAction::LeftClickDrag {
                start_coordinate: [1, 2],
                coordinate: [3, 4],
            },
        ),
        (
            json!({"action": "scroll", "scroll_direction": "down", "scroll_amount": 3}),
            ComputerAction::Scroll {
                coordinate: None,
                scroll_direction: ScrollDirection::Down,
                scroll_amount: 3,
            },
        ),
        (
            json!({"action": "key", "text": "ctrl+a", "repeat": 2}),
            ComputerAction::Key {
                text: "ctrl+a".to_string(),
                repeat: Some(2),
            },
        ),
        (
            json!({"action": "wait", "duration": 0.5}),
            ComputerAction::Wait { duration: 0.5 },
        ),
    ];
    for (args, expected) in cases {
        assert_eq!(ComputerAction::from_arguments(&args).unwrap(), expected);
    }
}

#[test]
fn extra_fields_are_ignored_and_bad_actions_are_named() {
    let with_intent = json!({"action": "screenshot", "human_intent": "Looking at the page"});
    assert_eq!(
        ComputerAction::from_arguments(&with_intent).unwrap(),
        ComputerAction::Screenshot
    );

    let err = ComputerAction::from_arguments(&json!({"action": "teleport"})).unwrap_err();
    assert!(err.contains("teleport"), "{err}");

    let err = ComputerAction::from_arguments(&json!({"action": "mouse_move"})).unwrap_err();
    assert!(err.contains("coordinate"), "{err}");
}

#[test]
fn validate_rejects_off_screen_and_out_of_range_values() {
    let display = DisplaySize {
        width: 100,
        height: 50,
    };
    let off = ComputerAction::LeftClick {
        coordinate: Some([100, 10]),
        text: None,
    };
    assert!(off.validate(display).unwrap_err().contains("outside"));

    let drag = ComputerAction::LeftClickDrag {
        start_coordinate: [0, 0],
        coordinate: [10, 50],
    };
    assert!(drag.validate(display).is_err());

    let scroll = ComputerAction::Scroll {
        coordinate: None,
        scroll_direction: ScrollDirection::Up,
        scroll_amount: 0,
    };
    assert!(scroll.validate(display).is_err());

    let wait = ComputerAction::Wait { duration: 31.0 };
    assert!(wait.validate(display).is_err());

    let bad_modifier = ComputerAction::LeftClick {
        coordinate: None,
        text: Some("hyper".to_string()),
    };
    assert!(
        bad_modifier
            .validate(display)
            .unwrap_err()
            .contains("hyper")
    );

    let ok = ComputerAction::DoubleClick {
        coordinate: Some([99, 49]),
        text: Some("ctrl+shift".to_string()),
    };
    assert!(ok.validate(display).is_ok());
}

#[test]
fn key_combos_split_modifiers_from_the_key() {
    let combo = parse_key_combo("ctrl+shift+t").unwrap();
    assert_eq!(combo.modifiers, vec![Modifier::Ctrl, Modifier::Shift]);
    assert_eq!(combo.key, "t");

    assert_eq!(parse_key_combo("Return").unwrap().key, "Return");
    assert_eq!(parse_key_combo("+").unwrap().key, "+");
    assert!(parse_key_combo("hyper+a").is_err());
    assert!(parse_key_combo("ctrl+").is_err());
}

#[test]
fn approval_gates_committing_actions_only() {
    let gated = [
        json!({"action": "type", "text": "hello"}),
        json!({"action": "key", "text": "Return"}),
        json!({"action": "key", "text": "ctrl+Enter"}),
        json!({"action": "navigate", "url": "https://example.com"}),
    ];
    for args in gated {
        let action = ComputerAction::from_arguments(&args).unwrap();
        assert!(action_requires_approval(&action), "{args}");
    }
    let free = [
        json!({"action": "screenshot"}),
        json!({"action": "left_click", "coordinate": [1, 1]}),
        json!({"action": "key", "text": "Tab"}),
        json!({"action": "scroll", "scroll_direction": "down", "scroll_amount": 1}),
    ];
    for args in free {
        let action = ComputerAction::from_arguments(&args).unwrap();
        assert!(!action_requires_approval(&action), "{args}");
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[test]
fn config_defaults_and_bounds() {
    assert_eq!(
        ComputerUseConfig::from_value(&Value::Null).unwrap(),
        ComputerUseConfig::default()
    );
    let custom = ComputerUseConfig::from_value(&json!({"display_width": 1024})).unwrap();
    assert_eq!(custom.display_width, 1024);
    assert_eq!(custom.display_height, DEFAULT_DISPLAY_HEIGHT);

    assert!(ComputerUseConfig::from_value(&json!({"display_width": 4000})).is_err());
    assert!(ComputerUseConfig::from_value(&json!({"max_actions_per_session": 0})).is_err());
    assert!(ComputerUseConfig::from_value(&json!({"colour": "red"})).is_err());
    assert_eq!(
        ComputerUseConfig::from_value_or_default(&json!({"display_width": 4000})),
        ComputerUseConfig::default()
    );
}

// ---------------------------------------------------------------------------
// Tool
// ---------------------------------------------------------------------------

#[test]
fn schema_offers_navigate_only_when_the_backend_supports_it() {
    let (browser, _) = tool_with(ComputerUseConfig::default(), true);
    let (desktop, _) = tool_with(ComputerUseConfig::default(), false);
    let actions =
        |tool: &ComputerTool| tool.parameters_schema()["properties"]["action"]["enum"].clone();
    assert!(
        actions(&browser)
            .as_array()
            .unwrap()
            .contains(&json!("navigate"))
    );
    assert!(
        !actions(&desktop)
            .as_array()
            .unwrap()
            .contains(&json!("navigate"))
    );
    assert!(desktop.description().contains("1280x800"));

    let hints = desktop.hints();
    assert_eq!(hints.open_world, Some(true));
    assert_eq!(hints.concurrency_class.as_deref(), Some(COMPUTER_TOOL_NAME));
}

#[tokio::test]
async fn an_action_runs_then_returns_a_screenshot() {
    let (tool, recorder) = tool_with(ComputerUseConfig::default(), false);
    let result = tool
        .execute_with_context(
            json!({"action": "left_click", "coordinate": [5, 6]}),
            &context(),
        )
        .await;

    match result {
        ToolExecutionResult::SuccessWithImages { result, images } => {
            assert_eq!(result["action"], "left_click");
            assert_eq!(result["backend"], "fake");
            assert_eq!(result["actions_used"], 1);
            assert_eq!(images.len(), 1);
            assert_eq!(images[0].media_type, "image/png");
        }
        other => panic!("expected an image result, got {other:?}"),
    }
    assert_eq!(
        *recorder.performed.lock().unwrap(),
        vec![ComputerAction::LeftClick {
            coordinate: Some([5, 6]),
            text: None
        }]
    );
    assert_eq!(*recorder.screenshots.lock().unwrap(), 1);
    assert_eq!(*recorder.releases.lock().unwrap(), 1);
    assert_eq!(
        *recorder.acquired_at.lock().unwrap(),
        vec![DisplaySize::default()]
    );
}

#[tokio::test]
async fn screenshot_after_action_off_returns_text_only() {
    let config = ComputerUseConfig {
        screenshot_after_action: false,
        ..ComputerUseConfig::default()
    };
    let (tool, recorder) = tool_with(config, false);
    let ctx = context();

    let result = tool
        .execute_with_context(json!({"action": "type", "text": "hi"}), &ctx)
        .await;
    assert!(
        matches!(result, ToolExecutionResult::Success(_)),
        "{result:?}"
    );
    assert_eq!(*recorder.screenshots.lock().unwrap(), 0);

    // The explicit screenshot action still returns an image.
    let result = tool
        .execute_with_context(json!({"action": "screenshot"}), &ctx)
        .await;
    assert!(matches!(
        result,
        ToolExecutionResult::SuccessWithImages { .. }
    ));
    assert_eq!(*recorder.screenshots.lock().unwrap(), 1);
}

#[tokio::test]
async fn invalid_actions_never_reach_the_backend_or_the_budget() {
    let (tool, recorder) = tool_with(ComputerUseConfig::default(), false);
    let ctx = context();

    for args in [
        json!({"action": "left_click", "coordinate": [5000, 1]}),
        json!({"action": "navigate", "url": "https://example.com"}),
        json!({"action": "fly"}),
    ] {
        let result = tool.execute_with_context(args.clone(), &ctx).await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(_)),
            "{args}: {result:?}"
        );
    }
    assert!(recorder.acquired_at.lock().unwrap().is_empty());
    let used = ctx
        .storage_store
        .as_ref()
        .unwrap()
        .get_value(ctx.session_id, ACTION_COUNT_KEY)
        .await
        .unwrap();
    assert_eq!(used, None);
}

#[tokio::test]
async fn the_session_action_budget_is_a_hard_cap() {
    let config = ComputerUseConfig {
        max_actions_per_session: 2,
        ..ComputerUseConfig::default()
    };
    let (tool, recorder) = tool_with(config, false);
    let ctx = context();

    for _ in 0..2 {
        let result = tool
            .execute_with_context(json!({"action": "screenshot"}), &ctx)
            .await;
        assert!(result.is_success());
    }
    let result = tool
        .execute_with_context(json!({"action": "screenshot"}), &ctx)
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => assert!(msg.contains("budget"), "{msg}"),
        other => panic!("expected budget error, got {other:?}"),
    }
    assert_eq!(recorder.acquired_at.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_failed_action_is_reported_and_the_display_released() {
    let recorder = Arc::new(Recorder::default());
    let backend = Arc::new(FakeBackend {
        recorder: recorder.clone(),
        navigation: false,
        fail_perform: true,
    });
    let tool = ComputerTool::new(backend, ComputerUseConfig::default());

    let result = tool
        .execute_with_context(
            json!({"action": "mouse_move", "coordinate": [1, 1]}),
            &context(),
        )
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("mouse_move failed: element moved"), "{msg}")
        }
        other => panic!("expected tool error, got {other:?}"),
    }
    assert_eq!(*recorder.releases.lock().unwrap(), 1);
    assert_eq!(*recorder.screenshots.lock().unwrap(), 0);
}

#[tokio::test]
async fn without_context_the_tool_refuses() {
    let (tool, _) = tool_with(ComputerUseConfig::default(), false);
    assert!(tool.requires_context());
    let result = tool.execute(json!({"action": "screenshot"})).await;
    assert!(matches!(result, ToolExecutionResult::ToolError(_)));
}
