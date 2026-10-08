use super::*;
use crate::error::Result as StoreResult;
use crate::runtime::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
use crate::typed_id::SessionId;
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
    /// Fail only actions with this name.
    fail_action: Option<&'static str>,
}

struct FakeSession {
    recorder: Arc<Recorder>,
    display: DisplaySize,
    fail_perform: bool,
    fail_action: Option<&'static str>,
}

#[async_trait]
impl ComputerSession for FakeSession {
    fn display(&self) -> DisplaySize {
        self.display
    }

    async fn perform(&mut self, action: &ComputerAction) -> Result<(), String> {
        if self.fail_perform || self.fail_action == Some(action.name()) {
            return Err("element moved".to_string());
        }
        self.recorder.performed.lock().unwrap().push(action.clone());
        Ok(())
    }

    async fn zoom(&mut self, region: [u32; 4]) -> Result<Screenshot, String> {
        self.recorder
            .performed
            .lock()
            .unwrap()
            .push(ComputerAction::Zoom { region });
        Ok(Screenshot {
            base64: "zoomed".to_string(),
            media_type: "image/png".to_string(),
        })
    }

    async fn cursor_position(&mut self) -> Result<[u32; 2], String> {
        Ok([7, 8])
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
            fail_action: self.fail_action,
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
        fail_action: None,
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
                text: None,
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
        text: None,
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
fn calls_parse_single_or_batched() {
    let single = ComputerCall::from_arguments(&json!({"action": "screenshot"})).unwrap();
    assert_eq!(single, ComputerCall::Single(ComputerAction::Screenshot));

    let batch = ComputerCall::from_arguments(&json!({"actions": [
        {"action": "left_click", "coordinate": [1, 2]},
        {"action": "type", "text": "Ada"}
    ]}))
    .unwrap();
    assert_eq!(batch.actions().len(), 2);
    assert_eq!(batch.actions()[1].name(), "type");

    let too_many: Vec<Value> = (0..=MAX_BATCH_ACTIONS)
        .map(|_| json!({"action": "screenshot"}))
        .collect();
    for bad in [
        json!({"actions": []}),
        json!({"actions": too_many}),
        json!({"actions": "screenshot"}),
        json!({"actions": [{"action": "fly"}]}),
        json!({}),
    ] {
        assert!(ComputerCall::from_arguments(&bad).is_err(), "{bad}");
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
    assert!(ComputerUseConfig::default().native_tools);
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
        .get_value(ctx.session_id, COMPUTER_USE_ACTION_COUNT_KEY)
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
        fail_action: None,
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

#[test]
fn native_tools_request_a_native_adapter_unless_turned_off() {
    let options = ComputerUseConfig {
        display_width: 1024,
        ..ComputerUseConfig::default()
    }
    .driver_options();
    assert_eq!(options.len(), 1);
    let options: std::collections::HashMap<_, _> = options.into_iter().collect();
    let native = crate::native_computer::NativeComputerUse::from_driver_options(&options)
        .expect("option parses");
    assert_eq!(native.display_width, 1024);
    assert_eq!(native.display_height, DEFAULT_DISPLAY_HEIGHT);

    let off = ComputerUseConfig::from_value(&json!({"native_tools": false})).unwrap();
    assert!(off.driver_options().is_empty());
}

#[tokio::test]
async fn a_batch_runs_in_order_charges_each_action_and_returns_one_frame() {
    let config = ComputerUseConfig {
        screenshot_after_action: false,
        ..ComputerUseConfig::default()
    };
    let (tool, recorder) = tool_with(config, false);
    let ctx = context();
    let args = json!({"actions": [
        {"action": "left_click", "coordinate": [10, 20]},
        {"action": "type", "text": "Ada"},
        {"action": "key", "text": "Tab"}
    ]});
    // The batch shape passes the tool's own argument schema.
    let validator = jsonschema::validator_for(&tool.parameters_schema()).unwrap();
    assert!(validator.is_valid(&args));

    let result = tool.execute_with_context(args, &ctx).await;
    let ToolExecutionResult::SuccessWithImages { result, images } = result else {
        panic!("a batch always answers with a frame: {result:?}");
    };
    assert_eq!(images.len(), 1);
    assert_eq!(result["actions"], json!(["left_click", "type", "key"]));
    assert_eq!(result["actions_used"], 3);
    let performed: Vec<&str> = recorder
        .performed
        .lock()
        .unwrap()
        .iter()
        .map(ComputerAction::name)
        .collect();
    assert_eq!(performed, ["left_click", "type", "key"]);
}

#[tokio::test]
async fn a_batch_stops_at_its_first_failure() {
    let recorder = Arc::new(Recorder::default());
    let backend = Arc::new(FakeBackend {
        recorder: recorder.clone(),
        navigation: false,
        fail_perform: false,
        fail_action: Some("type"),
    });
    let tool = ComputerTool::new(backend, ComputerUseConfig::default());

    let result = tool
        .execute_with_context(
            json!({"actions": [
                {"action": "left_click", "coordinate": [10, 20]},
                {"action": "type", "text": "Ada"},
                {"action": "key", "text": "Return"}
            ]}),
            &context(),
        )
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("action 2 of 3 (type) failed"), "{msg}")
        }
        other => panic!("expected tool error, got {other:?}"),
    }
    let performed = recorder.performed.lock().unwrap();
    assert_eq!(performed.len(), 1, "nothing after the failure runs");
    assert_eq!(*recorder.releases.lock().unwrap(), 1);
}

#[tokio::test]
async fn a_batch_is_validated_and_budgeted_whole() {
    let config = ComputerUseConfig {
        max_actions_per_session: 2,
        ..ComputerUseConfig::default()
    };
    let (tool, recorder) = tool_with(config, false);
    let ctx = context();

    // One off-screen action rejects the whole batch before anything runs.
    let result = tool
        .execute_with_context(
            json!({"actions": [
                {"action": "left_click", "coordinate": [1, 1]},
                {"action": "left_click", "coordinate": [9000, 1]}
            ]}),
            &ctx,
        )
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => assert!(msg.contains("action 2 of 2"), "{msg}"),
        other => panic!("expected validation error, got {other:?}"),
    }

    // Three actions do not fit a budget of two: nothing is charged or run.
    let three = json!({"actions": [
        {"action": "screenshot"}, {"action": "screenshot"}, {"action": "screenshot"}
    ]});
    match tool.execute_with_context(three, &ctx).await {
        ToolExecutionResult::ToolError(msg) => assert!(msg.contains("needs 3"), "{msg}"),
        other => panic!("expected budget error, got {other:?}"),
    }
    assert!(recorder.acquired_at.lock().unwrap().is_empty());
    let two = json!({"actions": [{"action": "screenshot"}, {"action": "screenshot"}]});
    assert!(tool.execute_with_context(two, &ctx).await.is_success());
}

#[test]
fn png_frames_are_checked_against_the_display_size() {
    use base64::Engine;
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&1280u32.to_be_bytes());
    bytes.extend_from_slice(&800u32.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let display = DisplaySize {
        width: 1280,
        height: 800,
    };
    assert_eq!(png_dimensions(&bytes).unwrap(), (1280, 800));
    let shot = png_screenshot(&bytes, display).unwrap();
    assert_eq!(shot.media_type, "image/png");

    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let from_base64 = png_base64_screenshot(encoded.clone(), display).unwrap();
    assert_eq!(from_base64.base64, encoded);

    let small = DisplaySize {
        width: 1024,
        height: 768,
    };
    let err = png_base64_screenshot(encoded, small).unwrap_err();
    assert!(err.contains("1280x800"), "{err}");
    assert!(png_base64_screenshot("not base64!".to_string(), display).is_err());
    assert!(png_screenshot(b"\xFF\xD8\xFF\xE0 jpeg", display).is_err());
}

#[test]
fn the_anthropic_toolset_members_all_parse() {
    let display = DisplaySize::default();
    for arguments in [
        json!({"action": "left_mouse_down"}),
        json!({"action": "left_mouse_up"}),
        json!({"action": "cursor_position"}),
        json!({"action": "zoom", "region": [0, 0, 100, 50]}),
        json!({"action": "hold_key", "text": "shift", "duration": 1.5}),
        json!({"action": "left_click_drag", "start_coordinate": [1, 2], "coordinate": [3, 4], "text": "alt"}),
    ] {
        let action = ComputerAction::from_arguments(&arguments).unwrap();
        assert_eq!(action.name(), arguments["action"], "{arguments}");
        action.validate(display).unwrap();
    }
}

#[test]
fn zoom_and_hold_key_are_validated() {
    let display = DisplaySize {
        width: 100,
        height: 100,
    };
    for region in [[10, 10, 10, 20], [0, 0, 101, 50], [20, 0, 10, 50]] {
        let zoom = ComputerAction::Zoom { region };
        assert!(zoom.validate(display).is_err(), "{region:?}");
    }
    let hold = |text: &str, duration: f64| ComputerAction::HoldKey {
        text: text.to_string(),
        duration,
    };
    assert!(hold("shift", -1.0).validate(display).is_err());
    assert!(hold("shift", 1000.0).validate(display).is_err());
    assert!(hold("", 1.0).validate(display).is_err());
    let drag = ComputerAction::LeftClickDrag {
        start_coordinate: [0, 0],
        coordinate: [1, 1],
        text: Some("bogus".to_string()),
    };
    assert!(drag.validate(display).is_err());
    assert_eq!(zoom_factor([0, 0, 50, 25], display), 2.0);
}

#[tokio::test]
async fn zoom_and_cursor_position_answer_without_a_screenshot() {
    let (tool, recorder) = tool_with(ComputerUseConfig::default(), false);
    let context = context();
    match tool
        .execute_with_context(
            json!({"action": "zoom", "region": [0, 0, 64, 40]}),
            &context,
        )
        .await
    {
        ToolExecutionResult::SuccessWithImages { result, images } => {
            assert_eq!(result["action"], "zoom");
            assert_eq!(images.len(), 1);
            assert_eq!(images[0].base64, "zoomed");
        }
        other => panic!("expected the zoomed region, got {other:?}"),
    }
    match tool
        .execute_with_context(json!({"action": "cursor_position"}), &context)
        .await
    {
        ToolExecutionResult::Success(result) => {
            assert_eq!(result["text"], "X=7, Y=8");
            assert_eq!(result["cursor_position"], json!({"x": 7, "y": 8}));
        }
        other => panic!("expected text, got {other:?}"),
    }
    assert_eq!(*recorder.screenshots.lock().unwrap(), 0);

    // In a batch the zoom comes before the closing frame.
    match tool
        .execute_with_context(
            json!({"actions": [{"action": "zoom", "region": [0, 0, 64, 40]}, {"action": "left_mouse_down"}]}),
            &context,
        )
        .await
    {
        ToolExecutionResult::SuccessWithImages { images, .. } => {
            assert_eq!(images.len(), 2);
            assert_eq!(images[0].base64, "zoomed");
        }
        other => panic!("expected two images, got {other:?}"),
    }
}

#[tokio::test]
async fn a_native_batch_stops_at_its_first_failed_call() {
    let recorder = Arc::new(Recorder::default());
    let backend = Arc::new(FakeBackend {
        recorder: recorder.clone(),
        navigation: false,
        fail_perform: false,
        fail_action: Some("type"),
    });
    let tool = ComputerTool::new(backend, ComputerUseConfig::default());
    let context = context();
    let call = |action: Value, batch: &str| {
        let mut action = action;
        action["native_batch"] = json!({ "id": batch });
        action
    };

    let failed = tool
        .execute_with_context(
            call(json!({"action": "type", "text": "Ada"}), "b1"),
            &context,
        )
        .await;
    assert!(
        matches!(failed, ToolExecutionResult::ToolError(_)),
        "{failed:?}"
    );

    let skipped = tool
        .execute_with_context(
            call(json!({"action": "key", "text": "Return"}), "b1"),
            &context,
        )
        .await;
    match skipped {
        ToolExecutionResult::ToolError(msg) => {
            assert_eq!(msg, crate::native_computer::NATIVE_BATCH_SKIPPED)
        }
        other => panic!("expected the skip text, got {other:?}"),
    }
    assert!(recorder.performed.lock().unwrap().is_empty());

    // The next turn's batch runs.
    let next = tool
        .execute_with_context(
            call(json!({"action": "key", "text": "Return"}), "b2"),
            &context,
        )
        .await;
    assert!(
        matches!(next, ToolExecutionResult::SuccessWithImages { .. }),
        "{next:?}"
    );
    assert_eq!(recorder.performed.lock().unwrap().len(), 1);
}
