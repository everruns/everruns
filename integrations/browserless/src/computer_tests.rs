use super::*;
use base64::Engine;
use everruns_contracts::typed_id::SessionId;
use everruns_core::capabilities::Capability;
use everruns_core::network_access::NetworkAccessList;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

// ---------------------------------------------------------------------------
// Pure mapping
// ---------------------------------------------------------------------------

#[test]
fn xdotool_names_map_to_cdp_keys() {
    let enter = cdp_key("Return");
    assert_eq!((enter.key.as_str(), enter.virtual_key), ("Enter", 13));
    assert_eq!(enter.text.as_deref(), Some("\r"));

    assert_eq!(cdp_key("Page_Down").key, "PageDown");
    assert_eq!(cdp_key("F5").virtual_key, 116);
    assert_eq!(cdp_key("BackSpace").code, "Backspace");

    let a = cdp_key("a");
    assert_eq!((a.code.as_str(), a.virtual_key), ("KeyA", 65));
    assert_eq!(a.text.as_deref(), Some("a"));
    assert_eq!(cdp_key("7").code, "Digit7");
}

#[test]
fn modifier_mask_matches_cdp_bits() {
    assert_eq!(modifier_mask(&[]), 0);
    assert_eq!(modifier_mask(&[Modifier::Alt]), 1);
    assert_eq!(modifier_mask(&[Modifier::Ctrl, Modifier::Shift]), 10);
    assert_eq!(
        modifier_mask(&[
            Modifier::Alt,
            Modifier::Ctrl,
            Modifier::Super,
            Modifier::Shift
        ]),
        15
    );
}

#[test]
fn url_policy_blocks_private_hosts_and_the_network_access_list() {
    let mut context = ToolContext::new(SessionId::new());
    assert!(url_blocked(&context, "http://127.0.0.1:8080/admin").is_some());
    assert!(url_blocked(&context, "http://169.254.169.254/latest").is_some());
    assert!(url_blocked(&context, "https://example.com/").is_none());

    context.network_access = Some(NetworkAccessList::allow_only(["example.com"]));
    assert!(url_blocked(&context, "https://example.com/form").is_none());
    let reason = url_blocked(&context, "https://evil.test/").unwrap();
    assert!(reason.contains("network access"), "{reason}");

    assert!(is_local_page("about:blank"));
    assert!(is_local_page("data:text/html,hi"));
    assert!(!is_local_page("https://example.com/"));
}

#[test]
fn capability_exposes_one_configured_computer_tool() {
    let cap = BrowserlessComputerUseCapability;
    assert_eq!(cap.id(), "computer_use");
    assert_eq!(cap.dependencies(), vec!["session_storage"]);
    assert!(
        cap.system_prompt_addition()
            .unwrap()
            .contains("untrusted data")
    );

    let tools = cap.tools_with_config(&json!({"display_width": 1024, "display_height": 768}));
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name(), "computer");
    assert!(tools[0].description().contains("1024x768"));
    assert!(tools[0].requires_context());
    // A browser display can navigate.
    let actions = tools[0].parameters_schema()["properties"]["action"]["enum"].clone();
    assert!(actions.as_array().unwrap().contains(&json!("navigate")));

    assert!(
        cap.validate_config(&json!({"display_width": 9000}))
            .is_err()
    );
    assert!(cap.validate_config(&json!({})).is_ok());
}

#[test]
fn capability_requests_native_tools_unless_turned_off() {
    let cap = BrowserlessComputerUseCapability;
    let options = cap.driver_options(&json!({"display_width": 1024, "display_height": 768}));
    assert_eq!(
        options,
        vec![(
            "everruns/computer_use".to_string(),
            json!({"display_width": 1024, "display_height": 768})
        )]
    );
    assert!(
        cap.driver_options(&json!({"native_tools": false}))
            .is_empty()
    );
}

#[tokio::test]
async fn without_a_browserless_connection_the_tool_says_how_to_connect() {
    let cap = BrowserlessComputerUseCapability;
    let tool = cap.tools().remove(0);
    let result = tool
        .execute_with_context(
            json!({"action": "screenshot"}),
            &ToolContext::new(SessionId::new()),
        )
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => assert!(msg.contains("Browserless"), "{msg}"),
        other => panic!("expected a connection error, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Real browser: the CDP action layer against a local headless Chromium
// ---------------------------------------------------------------------------

/// A local Chromium to drive, if this machine has one. CI runners ship Google
/// Chrome; cloud agent containers ship Playwright's Chromium.
fn chromium_binary() -> Option<String> {
    if let Ok(path) = std::env::var("CHROMIUM_PATH")
        && !path.is_empty()
    {
        return Some(path);
    }
    let fixed = [
        "/opt/pw-browsers/chromium-1194/chrome-linux/chrome",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ];
    if let Some(path) = fixed.iter().find(|p| std::path::Path::new(p).exists()) {
        return Some((*path).to_string());
    }
    // Any Playwright chromium build.
    std::fs::read_dir("/opt/pw-browsers")
        .ok()?
        .flatten()
        .map(|entry| entry.path().join("chrome-linux/chrome"))
        .find(|path| path.exists())
        .map(|path| path.to_string_lossy().into_owned())
}

struct LocalChromium {
    child: Child,
    ws_url: String,
    _profile: tempdir::Profile,
}

mod tempdir {
    /// A throwaway profile directory, removed on drop.
    pub struct Profile(pub std::path::PathBuf);

    impl Profile {
        pub fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "everruns-computer-use-{}",
                everruns_contracts::typed_id::SessionId::new()
            ));
            std::fs::create_dir_all(&dir).expect("create profile dir");
            Self(dir)
        }
    }

    impl Drop for Profile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

async fn launch_chromium() -> Option<LocalChromium> {
    let binary = chromium_binary()?;
    let profile = tempdir::Profile::new();
    let mut child = Command::new(&binary)
        .args([
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--no-first-run",
            "--no-default-browser-check",
            "--remote-debugging-port=0",
            "about:blank",
        ])
        .arg(format!("--user-data-dir={}", profile.0.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let stderr = child.stderr.take()?;
    let mut lines = BufReader::new(stderr).lines();
    let ws_url = tokio::time::timeout(Duration::from_secs(20), async {
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(url) = line.strip_prefix("DevTools listening on ") {
                return Some(url.trim().to_string());
            }
        }
        None
    })
    .await
    .ok()??;
    // Keep draining stderr so Chromium never blocks on a full pipe.
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    Some(LocalChromium {
        child,
        ws_url,
        _profile: profile,
    })
}

const FORM_PAGE: &str = r#"<!doctype html>
<html><body style="margin:0;font:16px sans-serif;height:3000px">
<form id="f" style="padding:20px">
  <input id="name" style="display:block;width:300px;height:30px;margin:10px">
  <input id="email" style="display:block;width:300px;height:30px;margin:10px">
  <button id="go" style="display:block;width:120px;height:30px;margin:10px">Send</button>
</form>
<p style="margin:20px"><span id="word">alpha beta</span></p>
<div id="hover" style="margin:20px;width:100px;height:40px;background:#ccc"></div>
<p id="result"></p>
<script>
  document.getElementById('f').addEventListener('submit', (e) => {
    e.preventDefault();
    const name = document.getElementById('name').value;
    const email = document.getElementById('email').value;
    document.getElementById('result').textContent = 'Thanks, ' + name + ' <' + email + '>';
  });
  document.getElementById('hover').addEventListener('mouseover', () => { window.hovered = true; });
  document.getElementById('hover').addEventListener('contextmenu', (e) => {
    e.preventDefault(); window.contextMenus = (window.contextMenus || 0) + 1;
  });
</script>
</body></html>"#;

async fn eval(display: &mut CdpDisplay, expression: &str) -> Value {
    let result = display.session_mut().evaluate(expression).await.unwrap();
    result["result"]["value"].clone()
}

/// Center of an element, as a model would read it off the screenshot.
async fn center(display: &mut CdpDisplay, selector: &str) -> [u32; 2] {
    let rect = eval(
        display,
        &format!(
            "(() => {{ const r = document.querySelector('{selector}').getBoundingClientRect(); \
             return [Math.round(r.x + r.width / 2), Math.round(r.y + r.height / 2)]; }})()"
        ),
    )
    .await;
    [
        rect[0].as_u64().unwrap() as u32,
        rect[1].as_u64().unwrap() as u32,
    ]
}

fn action(value: Value) -> ComputerAction {
    ComputerAction::from_arguments(&value).unwrap()
}

#[tokio::test]
async fn fills_and_submits_a_form_on_a_real_browser() {
    let Some(mut browser) = launch_chromium().await else {
        eprintln!("skipping: no local Chromium found (set CHROMIUM_PATH to run)");
        return;
    };
    // Test builds use a 1s CDP connect timeout; a freshly started Chromium on
    // a loaded CI runner can take longer to accept the socket, so retry.
    let mut session = None;
    for _ in 0..20 {
        match CdpSession::connect(&browser.ws_url).await {
            Ok(connected) => {
                session = Some(connected);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
    let session = session.expect("connect to local Chromium over CDP");
    let size = DisplaySize {
        width: 800,
        height: 600,
    };
    let mut display = CdpDisplay::attach(session, size, [0, 0])
        .await
        .map_err(|(_, e)| e)
        .unwrap();

    let url = format!(
        "data:text/html;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(FORM_PAGE)
    );
    display
        .perform(&ComputerAction::Navigate { url })
        .await
        .unwrap();

    // The viewport is exactly the configured display.
    let shot = display.screenshot().await.unwrap();
    assert_eq!(shot.media_type, "image/png");
    let png = base64::engine::general_purpose::STANDARD
        .decode(&shot.base64)
        .unwrap();
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert_eq!((width, height), (800, 600));

    // Click into the name field, type, Tab to email, type, press Enter.
    let [x, y] = center(&mut display, "#name").await;
    display
        .perform(&action(
            json!({"action": "left_click", "coordinate": [x, y]}),
        ))
        .await
        .unwrap();
    assert_eq!(display.cursor(), [x, y]);
    display
        .perform(&action(json!({"action": "type", "text": "Ada Lovelace"})))
        .await
        .unwrap();
    display
        .perform(&action(json!({"action": "key", "text": "Tab"})))
        .await
        .unwrap();
    display
        .perform(&action(
            json!({"action": "type", "text": "ada@example.com"}),
        ))
        .await
        .unwrap();
    // ctrl+a selects rather than typing "a"; retyping replaces the selection.
    display
        .perform(&action(json!({"action": "key", "text": "ctrl+a"})))
        .await
        .unwrap();
    display
        .perform(&action(
            json!({"action": "type", "text": "ada@lovelace.dev"}),
        ))
        .await
        .unwrap();
    display
        .perform(&action(json!({"action": "key", "text": "Return"})))
        .await
        .unwrap();
    assert_eq!(
        eval(
            &mut display,
            "document.getElementById('result').textContent"
        )
        .await,
        json!("Thanks, Ada Lovelace <ada@lovelace.dev>")
    );

    // Double click selects a word.
    let [wx, wy] = center(&mut display, "#word").await;
    display
        .perform(&action(
            json!({"action": "double_click", "coordinate": [wx - 20, wy]}),
        ))
        .await
        .unwrap();
    let selected = eval(&mut display, "window.getSelection().toString().trim()").await;
    assert_eq!(selected, json!("alpha"));

    // Hover and right click reach the element under the pointer.
    let hover = center(&mut display, "#hover").await;
    display
        .perform(&action(
            json!({"action": "mouse_move", "coordinate": hover}),
        ))
        .await
        .unwrap();
    assert_eq!(
        eval(&mut display, "window.hovered === true").await,
        json!(true)
    );
    display
        .perform(&action(
            json!({"action": "right_click", "coordinate": hover}),
        ))
        .await
        .unwrap();
    assert_eq!(eval(&mut display, "window.contextMenus").await, json!(1));

    // Scrolling moves the page by wheel clicks.
    display
        .perform(&action(json!({
            "action": "scroll", "coordinate": [400, 300],
            "scroll_direction": "down", "scroll_amount": 3
        })))
        .await
        .unwrap();
    // Wheel scrolling is applied asynchronously by the compositor.
    let mut scrolled = json!(0);
    for _ in 0..20 {
        scrolled = eval(&mut display, "window.scrollY").await;
        if scrolled.as_f64().unwrap_or(0.0) > 0.0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(scrolled.as_f64(), Some(300.0));

    display.into_session().disconnect().await;
    let _ = browser.child.kill().await;
}
