use super::*;
use everruns_contracts::runtime::capabilities::Capability;

fn action(value: Value) -> ComputerAction {
    ComputerAction::from_arguments(&value).unwrap()
}

fn inputs(value: Value) -> Vec<DesktopInput> {
    desktop_inputs(&action(value), Some([7, 9])).unwrap()
}

// ---------------------------------------------------------------------------
// Action -> Computer Use API requests
// ---------------------------------------------------------------------------

#[test]
fn clicks_send_button_count_and_modifiers() {
    let left = inputs(json!({"action": "left_click", "coordinate": [10, 20]}));
    assert_eq!(
        left,
        vec![input(
            "/mouse/click",
            json!({"x": 10, "y": 20, "button": "left", "clicks": 1, "double": false, "modifiers": []})
        )]
    );

    let right = inputs(json!({"action": "right_click", "coordinate": [1, 2]}));
    assert_eq!(right[0].body["button"], "right");

    let double = inputs(json!({"action": "double_click", "coordinate": [1, 2]}));
    assert_eq!(double[0].body["clicks"], 2);
    assert_eq!(double[0].body["double"], true);

    let triple = inputs(json!({
        "action": "triple_click", "coordinate": [1, 2], "text": "ctrl+super"
    }));
    assert_eq!(triple[0].body["clicks"], 3);
    assert_eq!(triple[0].body["modifiers"], json!(["ctrl", "cmd"]));
}

#[test]
fn clicks_and_scrolls_without_a_coordinate_act_at_the_pointer() {
    let click = action(json!({"action": "middle_click"}));
    assert!(needs_pointer(&click));
    let sent = desktop_inputs(&click, Some([7, 9])).unwrap();
    assert_eq!(
        (sent[0].body["x"].clone(), sent[0].body["y"].clone()),
        (json!(7), json!(9))
    );
    assert!(desktop_inputs(&click, None).is_err());

    let scroll = action(json!({
        "action": "scroll", "scroll_direction": "left", "scroll_amount": 4
    }));
    assert!(needs_pointer(&scroll));
    assert_eq!(
        desktop_inputs(&scroll, Some([3, 4])).unwrap(),
        vec![input(
            "/mouse/scroll",
            json!({"x": 3, "y": 4, "direction": "left", "amount": 4})
        )]
    );

    assert!(!needs_pointer(&action(
        json!({"action": "left_click", "coordinate": [1, 1]})
    )));
    assert!(!needs_pointer(&action(
        json!({"action": "type", "text": "x"})
    )));
}

#[test]
fn drag_and_move_use_their_own_endpoints() {
    let drag = inputs(json!({
        "action": "left_click_drag", "start_coordinate": [10, 10], "coordinate": [30, 50]
    }));
    assert_eq!(
        drag,
        vec![input(
            "/mouse/drag",
            json!({"startX": 10, "startY": 10, "endX": 30, "endY": 50, "button": "left"})
        )]
    );
    let hover = inputs(json!({"action": "mouse_move", "coordinate": [5, 6]}));
    assert_eq!(hover, vec![input("/mouse/move", json!({"x": 5, "y": 6}))]);
}

#[test]
fn a_modified_drag_holds_its_modifiers_around_the_drag() {
    let drag = inputs(json!({
        "action": "left_click_drag", "start_coordinate": [1, 2], "coordinate": [3, 4],
        "text": "shift+ctrl"
    }));
    let paths: Vec<(&str, Value)> = drag
        .iter()
        .map(|sent| {
            (
                sent.path,
                sent.body.get("key").cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    assert_eq!(
        paths,
        vec![
            ("/keyboard/down", json!("shift")),
            ("/keyboard/down", json!("ctrl")),
            ("/mouse/drag", Value::Null),
            ("/keyboard/up", json!("ctrl")),
            ("/keyboard/up", json!("shift")),
        ]
    );
}

#[test]
fn button_down_and_up_act_at_the_pointer() {
    assert_eq!(
        inputs(json!({"action": "left_mouse_down"})),
        vec![input("/mouse/down", json!({"button": "left"}))]
    );
    assert_eq!(
        inputs(json!({"action": "left_mouse_up"})),
        vec![input("/mouse/up", json!({"button": "left"}))]
    );
    for session_only in [
        json!({"action": "zoom", "region": [0, 0, 10, 10]}),
        json!({"action": "cursor_position"}),
        json!({"action": "hold_key", "text": "shift", "duration": 1.0}),
    ] {
        assert!(inputs(session_only.clone()).is_empty(), "{session_only}");
    }
}

#[test]
fn held_keys_press_modifiers_then_the_key() {
    assert_eq!(held_keys("shift").unwrap(), vec!["shift"]);
    assert_eq!(held_keys("ctrl+a").unwrap(), vec!["ctrl", "a"]);
    assert!(held_keys("").is_err());
}

#[test]
fn typed_text_is_one_json_string_even_with_shell_metacharacters() {
    let text = "$(rm -rf /); `id` | cat > /etc/passwd -- --help\n";
    let sent = inputs(json!({"action": "type", "text": text}));
    assert_eq!(sent, vec![input("/keyboard/type", json!({"text": text}))]);
}

#[test]
fn key_combos_map_to_daemon_key_names() {
    let sent = inputs(json!({"action": "key", "text": "ctrl+shift+t"}));
    assert_eq!(
        sent,
        vec![input(
            "/keyboard/key",
            json!({"key": "t", "modifiers": ["ctrl", "shift"]})
        )]
    );
    let enter = inputs(json!({"action": "key", "text": "Return", "repeat": 3}));
    assert_eq!(enter.len(), 3);
    assert_eq!(enter[2].body, json!({"key": "enter", "modifiers": []}));

    assert_eq!(
        daytona_key("Page_Up").unwrap(),
        ("pageup".to_string(), false)
    );
    assert_eq!(
        daytona_key("ArrowLeft").unwrap(),
        ("left".to_string(), false)
    );
    assert_eq!(daytona_key("F12").unwrap(), ("f12".to_string(), false));
    assert_eq!(daytona_key("A").unwrap(), ("a".to_string(), true));
    assert_eq!(daytona_key("!").unwrap(), ("1".to_string(), true));
    assert_eq!(daytona_key("+").unwrap(), ("=".to_string(), true));
    assert_eq!(daytona_key("plus").unwrap(), ("=".to_string(), true));
    assert_eq!(daytona_key("minus").unwrap(), ("-".to_string(), false));
    assert!(daytona_key("ctrl a").is_err());
}

#[test]
fn shifted_symbols_add_shift_once() {
    let sent = inputs(json!({"action": "key", "text": "shift+A"}));
    assert_eq!(sent[0].body, json!({"key": "a", "modifiers": ["shift"]}));
    let plus = inputs(json!({"action": "key", "text": "ctrl+plus"}));
    assert_eq!(
        plus[0].body,
        json!({"key": "=", "modifiers": ["ctrl", "shift"]})
    );
}

#[test]
fn a_bare_modifier_is_pressed_and_released_in_reverse() {
    let sent = inputs(json!({"action": "key", "text": "ctrl+alt"}));
    assert_eq!(
        sent,
        vec![
            input("/keyboard/down", json!({"key": "ctrl"})),
            input("/keyboard/down", json!({"key": "alt"})),
            input("/keyboard/up", json!({"key": "alt"})),
            input("/keyboard/up", json!({"key": "ctrl"})),
        ]
    );
    let super_key = inputs(json!({"action": "key", "text": "super"}));
    assert_eq!(super_key[0], input("/keyboard/down", json!({"key": "cmd"})));
}

#[test]
fn screenshot_and_wait_send_nothing_and_navigate_is_refused() {
    assert!(inputs(json!({"action": "screenshot"})).is_empty());
    assert!(inputs(json!({"action": "wait", "duration": 1})).is_empty());
    let err = desktop_inputs(
        &action(json!({"action": "navigate", "url": "https://example.com"})),
        None,
    )
    .unwrap_err();
    assert!(err.contains("not available on a desktop"));
}

#[test]
fn daemon_errors_read_as_their_message() {
    let raw = r#"Sandbox API error (500 Internal Server Error): {"statusCode":500,"message":"internal server error: unsupported key \"bogus\"; see supported keyboard key names","code":"INTERNAL_SERVER_ERROR"}"#;
    assert_eq!(
        readable_error(raw),
        "unsupported key \"bogus\"; see supported keyboard key names"
    );
    assert_eq!(readable_error("connection reset"), "connection reset");
}

#[test]
fn primary_display_prefers_the_active_one() {
    let info = json!({"displays": [
        {"id": 1, "width": 800, "height": 600, "isActive": false},
        {"id": 0, "width": 1280, "height": 800, "isActive": true}
    ]});
    assert_eq!(
        primary_display(&info),
        Some(DisplaySize {
            width: 1280,
            height: 800
        })
    );
    assert_eq!(primary_display(&json!({"displays": []})), None);
    assert_eq!(
        primary_display(&json!({"displays": [{"width": 0, "height": 0}]})),
        None
    );
}

// ---------------------------------------------------------------------------
// Capability
// ---------------------------------------------------------------------------

#[test]
fn capability_provides_the_desktop_computer_tool() {
    let cap = DaytonaDesktopComputerUseCapability;
    assert_eq!(cap.id(), DAYTONA_COMPUTER_USE_CAPABILITY_ID);
    assert_eq!(cap.exclusive_group(), Some(COMPUTER_TOOL_NAME));
    assert_eq!(cap.risk_level(), RiskLevel::High);
    assert!(cap.features().contains(&LEASED_RESOURCES_FEATURE));
    let tools = cap.tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name(), COMPUTER_TOOL_NAME);
    // A desktop has no navigate action.
    let schema = tools[0].parameters_schema();
    let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
    assert!(!actions.iter().any(|a| a == "navigate"));
    assert!(
        cap.system_prompt_addition()
            .unwrap()
            .contains("Linux desktop")
    );
    assert!(cap.validate_config(&json!({"display_width": 1024})).is_ok());
    assert!(
        cap.validate_config(&json!({"display_width": 99999}))
            .is_err()
    );
}

// ---------------------------------------------------------------------------
// HTTP: one session against a mock toolbox
// ---------------------------------------------------------------------------

mod http {
    use super::*;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const SANDBOX: &str = "sbx-1";

    fn png_base64(width: u32, height: u32) -> String {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        everruns_contracts::runtime::computer_use::png_screenshot(
            &bytes,
            DisplaySize { width, height },
        )
        .unwrap()
        .base64
    }

    fn route(suffix: &str) -> String {
        format!("/toolbox/{SANDBOX}/computeruse{suffix}")
    }

    async fn server() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(route("/status")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "active"})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(route("/display/info")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "displays": [{"id": 0, "width": 1024, "height": 768, "isActive": true}]
            })))
            .mount(&server)
            .await;
        server
    }

    fn client(server: &MockServer) -> DaytonaClient {
        DaytonaClient::with_base_urls(
            "key".to_string(),
            server.uri(),
            format!("{}/toolbox", server.uri()),
        )
    }

    const DISPLAY: DisplaySize = DisplaySize {
        width: 1024,
        height: 768,
    };

    #[tokio::test]
    async fn a_click_at_the_pointer_reads_the_position_first() {
        let server = server().await;
        Mock::given(method("GET"))
            .and(path(route("/mouse/position")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"x": 40, "y": 50})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(route("/mouse/click")))
            .and(body_json(json!({
                "x": 40, "y": 50, "button": "left", "clicks": 1, "double": false, "modifiers": []
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"x": 40, "y": 50})))
            .expect(1)
            .mount(&server)
            .await;

        let mut session =
            DaytonaDesktopSession::start(client(&server), SANDBOX.to_string(), DISPLAY)
                .await
                .unwrap();
        session
            .perform(&action(json!({"action": "left_click"})))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_screenshot_is_checked_against_the_display() {
        let server = server().await;
        Mock::given(method("GET"))
            .and(path(route("/screenshot")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"screenshot": png_base64(1024, 768)})),
            )
            .mount(&server)
            .await;
        let mut session =
            DaytonaDesktopSession::start(client(&server), SANDBOX.to_string(), DISPLAY)
                .await
                .unwrap();
        let shot = session.screenshot().await.unwrap();
        assert_eq!(shot.media_type, "image/png");
        assert_eq!(shot.base64, png_base64(1024, 768));
    }

    #[tokio::test]
    async fn a_desktop_of_another_size_is_refused() {
        let server = server().await;
        let other = DisplaySize {
            width: 1280,
            height: 800,
        };
        let err =
            match DaytonaDesktopSession::start(client(&server), SANDBOX.to_string(), other).await {
                Ok(_) => panic!("a 1024x768 desktop must not pass for 1280x800"),
                Err(err) => err,
            };
        assert!(err.contains("1024x768"), "{err}");
    }

    #[tokio::test]
    async fn an_unknown_key_reports_the_daemon_reason() {
        let server = server().await;
        Mock::given(method("POST"))
            .and(path(route("/keyboard/key")))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({
                "statusCode": 500,
                "message": "internal server error: unsupported key \"xf86audiomute\""
            })))
            .mount(&server)
            .await;
        let mut session =
            DaytonaDesktopSession::start(client(&server), SANDBOX.to_string(), DISPLAY)
                .await
                .unwrap();
        let err = session
            .perform(&action(json!({"action": "key", "text": "XF86AudioMute"})))
            .await
            .unwrap_err();
        assert_eq!(err, "unsupported key \"xf86audiomute\"");
    }

    #[tokio::test]
    async fn the_desktop_is_started_when_it_is_not_running() {
        let server = MockServer::start().await;
        // Status reports inactive until start runs.
        Mock::given(method("GET"))
            .and(path(route("/status")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "inactive"})))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(route("/status")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "active"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(route("/start")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"message": "ok"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(route("/display/info")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "displays": [{"id": 0, "width": 1024, "height": 768, "isActive": true}]
            })))
            .mount(&server)
            .await;
        DaytonaDesktopSession::start(client(&server), SANDBOX.to_string(), DISPLAY)
            .await
            .unwrap();
    }
}
