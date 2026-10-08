use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::*;
use crate::browserless::test_chromium::{connect_guarded, launch_chromium};
use base64::Engine;
use everruns_contracts::error::Result as StoreResult;
use everruns_contracts::runtime::capabilities::Capability;
use everruns_contracts::runtime::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
use everruns_contracts::typed_id::SessionId;

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

fn context() -> (ToolContext, Arc<MemoryStore>) {
    let storage = Arc::new(MemoryStore::default());
    (
        ToolContext::new(SessionId::new()).with_storage_store_arc(storage.clone()),
        storage,
    )
}

#[test]
fn navigation_takes_http_and_https_only() {
    for (input, expected) in [
        ("https://example.com/a", "https://example.com/a"),
        ("http://example.com", "http://example.com"),
        ("example.com/path", "https://example.com/path"),
        ("example.com:8080/x", "https://example.com:8080/x"),
        ("localhost:3000", "https://localhost:3000"),
        (" back ", "back"),
        ("reload", "reload"),
    ] {
        assert_eq!(test_navigation_target(input).as_deref(), Ok(expected));
    }
    for refused in [
        "file:///etc/passwd",
        "javascript:alert(1)",
        "data:text/html,hi",
        "chrome://settings",
        "about:blank",
    ] {
        assert_eq!(
            test_navigation_target(refused),
            Err("Navigation refused. Only http and https URLs are allowed.".into()),
            "{refused}"
        );
    }
}

#[tokio::test]
async fn refused_calls_never_touch_the_browser_or_the_budget() {
    let tool = BrowserUseCapability.tools().remove(0);
    let (context, storage) = context();
    for arguments in [
        json!({"action": "navigate", "url": "file:///etc/passwd"}),
        json!({"action": "javascript_exec", "text": "1"}),
        json!({"action": "left_click", "target": {"type": "coordinate", "x": 5000, "y": 1}}),
    ] {
        match tool.execute_with_context(arguments.clone(), &context).await {
            ToolExecutionResult::ToolError(_) => {}
            other => panic!("{arguments}: {other:?}"),
        }
    }
    assert!(storage.0.lock().unwrap().is_empty());
}

#[test]
fn capability_shape() {
    let cap = BrowserUseCapability;
    assert_eq!(cap.id(), "browser_use");
    assert_eq!(cap.dependencies(), vec!["session_storage"]);
    let tools = cap.tools();
    assert_eq!(tools[0].name(), "browser");
    assert!(cap.validate_config(&json!({"viewport_width": 10})).is_err());
    assert!(cap.validate_config(&json!({})).is_ok());
}

const FORM_PAGE: &str = r#"<!doctype html><html><head><title>Signup</title></head><body>
<main>
<h1>Create account</h1>
<input id="name" aria-label="Name">
<select id="color" aria-label="Color"><option value="r">Red</option><option value="b">Blue</option></select>
<input type="checkbox" id="news" aria-label="Newsletter">
<button onclick="window.submitted = document.getElementById('name').value">Sign up</button>
<div style="height: 3000px"></div>
<a href="https://example.com/privacy">Privacy policy</a>
</main>
<script>
  // A hostile page: its own value setter lies. form_input runs in an
  // isolated world and does not go through it.
  Object.defineProperty(HTMLInputElement.prototype, 'value', {
    set() { this.setAttribute('data-hijacked', '1'); },
    get() { return 'hijacked'; },
  });
</script>
</body></html>"#;

fn data_url(html: &str) -> String {
    format!(
        "data:text/html;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(html)
    )
}

async fn step(
    tool: &BrowserTool,
    context: &ToolContext,
    session: &mut Connected,
    arguments: Value,
) -> Result<Done, String> {
    let call = BrowserCall::from_arguments(&arguments).unwrap();
    tool.drive(&call, context, session, tool.config.viewport())
        .await
}

async fn ok(
    tool: &BrowserTool,
    context: &ToolContext,
    session: &mut Connected,
    arguments: Value,
) -> String {
    step(tool, context, session, arguments.clone())
        .await
        .unwrap_or_else(|e| panic!("{arguments}: {e}"))
        .text
}

/// The ref a line naming `name` carries.
fn ref_of(text: &str, name: &str) -> String {
    let line = text
        .lines()
        .find(|line| line.contains(&format!("\"{name}\"")))
        .unwrap_or_else(|| panic!("no {name} in\n{text}"));
    let start = line.rfind("[ref_").unwrap() + 1;
    line[start..line.len() - 1].to_string()
}

async fn page_eval(session: &mut Connected, expression: &str) -> Value {
    let result = session
        .display
        .session_mut()
        .evaluate(expression)
        .await
        .unwrap();
    result["result"]["value"].clone()
}

#[tokio::test]
async fn reads_acts_by_ref_and_manages_tabs_on_a_real_browser() {
    let Some(mut browser) = launch_chromium().await else {
        eprintln!("skipping: no local Chromium found (set CHROMIUM_PATH to run)");
        return;
    };
    let cdp = connect_guarded(&browser, BrowserEgress::new(None)).await;
    let (context, _) = context();
    let tool = BrowserTool::new(BrowserUseConfig::from_value(&json!({})).unwrap());
    let mut session = Connected {
        display: CdpDisplay::detached(cdp, [0, 0]),
    };
    session
        .display
        .session_mut()
        .navigate(&data_url(FORM_PAGE))
        .await
        .unwrap();

    // The viewport read shows the form but not the link far below it.
    let page = ok(
        &tool,
        &context,
        &mut session,
        json!({"action": "read_page"}),
    )
    .await;
    assert!(page.contains("heading \"Create account\""), "{page}");
    assert!(!page.contains("Privacy policy"), "{page}");
    let interactive = ok(
        &tool,
        &context,
        &mut session,
        json!({"action": "read_page", "filter": "interactive"}),
    )
    .await;
    let name = ref_of(&interactive, "Name");
    let color = ref_of(&interactive, "Color");
    let news = ref_of(&interactive, "Newsletter");
    let button = ref_of(&interactive, "Sign up");

    // find reaches the off-screen link; refs stay stable across reads.
    let found = ok(
        &tool,
        &context,
        &mut session,
        json!({"action": "find", "query": "privacy policy link"}),
    )
    .await;
    assert!(found.starts_with("link \"Privacy policy\""), "{found}");
    let again = ok(
        &tool,
        &context,
        &mut session,
        json!({"action": "find", "query": "sign up button"}),
    )
    .await;
    assert_eq!(ref_of(&again, "Sign up"), button);

    // form_input sets fields through the browser's own setters, not the page's.
    for (target, value) in [
        (&name, json!("Ada")),
        (&color, json!("Blue")),
        (&news, json!(true)),
    ] {
        ok(
            &tool,
            &context,
            &mut session,
            json!({"action": "form_input", "target": {"type": "ref", "ref": target}, "value": value}),
        )
        .await;
    }
    assert_eq!(
        page_eval(&mut session, "document.getElementById('color').value").await,
        json!("b")
    );
    assert_eq!(
        page_eval(&mut session, "document.getElementById('news').checked").await,
        json!(true)
    );
    assert_eq!(
        page_eval(
            &mut session,
            "document.getElementById('name').hasAttribute('data-hijacked')"
        )
        .await,
        json!(false)
    );
    let refused = step(
        &tool,
        &context,
        &mut session,
        json!({"action": "form_input", "target": {"type": "ref", "ref": button}, "value": "x"}),
    )
    .await
    .err()
    .unwrap();
    assert!(refused.contains("not a form field"), "{refused}");

    // A ref click lands on the element.
    ok(
        &tool,
        &context,
        &mut session,
        json!({"action": "left_click", "target": {"type": "ref", "ref": button}}),
    )
    .await;
    assert_eq!(
        page_eval(&mut session, "window.submitted !== undefined").await,
        json!(true)
    );

    let text = ok(
        &tool,
        &context,
        &mut session,
        json!({"action": "get_page_text"}),
    )
    .await;
    assert!(text.contains("Create account"), "{text}");

    // A new document makes every ref stale.
    session
        .display
        .session_mut()
        .navigate(&data_url("<button>Other</button>"))
        .await
        .unwrap();
    let stale = step(
        &tool,
        &context,
        &mut session,
        json!({"action": "left_click", "target": {"type": "ref", "ref": button}}),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(stale, page::stale_ref(&button));

    // Tabs: the first report lists the tab; later ones name new tabs.
    let first_tab = session.display.session_mut().page_target_id().to_string();
    let state = browser_state(&context, &mut session.display).await.unwrap();
    assert_eq!(state["tabs"].as_array().unwrap().len(), 1);
    assert_eq!(state["tabs"][0]["active"], json!(true));
    assert!(state.get("state_changes").is_none());

    let created = step(&tool, &context, &mut session, json!({"action": "new_tab"}))
        .await
        .unwrap();
    let second_tab = created.tab.clone().unwrap();
    assert_eq!(
        created.text,
        format!(
            "Created new tab with tab_id: {second_tab}, URL: about:blank. It is now the current tab."
        )
    );
    let state = browser_state(&context, &mut session.display).await.unwrap();
    assert_eq!(state["tabs"].as_array().unwrap().len(), 2);
    assert_eq!(
        state["state_changes"],
        json!([{ "type": "tab_opened", "tab_id": second_tab }])
    );
    let active: Vec<&Value> = state["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|tab| tab.get("active").is_some())
        .collect();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0]["tab_id"], json!(second_tab));
    let state = browser_state(&context, &mut session.display).await.unwrap();
    assert!(state.get("state_changes").is_none());

    let listed = ok(
        &tool,
        &context,
        &mut session,
        json!({"action": "list_tabs"}),
    )
    .await;
    assert!(listed.starts_with("Available tabs:"), "{listed}");
    assert!(listed.contains(&first_tab) && listed.contains(&second_tab));

    assert_eq!(
        ok(
            &tool,
            &context,
            &mut session,
            json!({"action": "switch_tab", "tab_id": first_tab}),
        )
        .await,
        format!("Switched to tab {first_tab}")
    );
    let other = ok(
        &tool,
        &context,
        &mut session,
        json!({"action": "read_page"}),
    )
    .await;
    assert!(other.contains("button \"Other\""), "{other}");

    // An action can name its tab; the active tab follows it.
    ok(
        &tool,
        &context,
        &mut session,
        json!({"action": "screenshot", "tab_id": second_tab}),
    )
    .await;
    assert_eq!(session.display.session_mut().page_target_id(), second_tab);
    assert!(
        step(
            &tool,
            &context,
            &mut session,
            json!({"action": "screenshot", "tab_id": "not-a-tab"}),
        )
        .await
        .is_err()
    );

    assert_eq!(
        ok(
            &tool,
            &context,
            &mut session,
            json!({"action": "close_tab", "tab_id": second_tab}),
        )
        .await,
        format!("Closed tab {second_tab}")
    );
    assert_eq!(session.display.session_mut().page_target_id(), first_tab);
    let state = browser_state(&context, &mut session.display).await.unwrap();
    assert_eq!(state["tabs"].as_array().unwrap().len(), 1);

    session.display.into_session().disconnect().await;
    let _ = browser.child.kill().await;
}
