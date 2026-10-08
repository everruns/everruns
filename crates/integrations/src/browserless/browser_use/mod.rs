//! Browser use on a Browserless browser: the provider-neutral `browser` tool.
//!
//! The vocabulary lives in [`everruns_contracts::runtime::browser_use`]. This
//! module runs it on the session's persistent Browserless browser, the same
//! one the `browserless_*` and `computer` tools use.
//!
//! Decision: pointer and keyboard actions reuse [`CdpDisplay`], so a
//! coordinate click here is the same CDP input as a `computer` click. A ref
//! target is resolved to the element's center after scrolling it into view,
//! then clicked like a coordinate.
//!
//! Decision: tabs are the page targets of the session's guarded browser
//! context, and a tab id is Chrome's target id. Which tab is active, the tabs
//! already reported, the cursor and each tab's refs live under the reserved
//! `browser_use.` session storage prefix, because every call reconnects.
//!
//! Decision: every successful call answers with a browser state report (each
//! tab's id, title and URL, which is active, and tabs opened since the last
//! report), the shape Anthropic's `browser_state` block takes.
//!
//! THREAT[TM-TOOL-015]: `navigate` takes only http and https URLs (an address
//! without a scheme is read as https) and goes through the shared SSRF
//! validator and the session network access list. After every action the
//! active page's URL is checked against the same policy and the page is sent
//! to `about:blank` if it left it, as the `computer` tool does. Every tab is
//! attached through the same `Fetch` guard.

pub(crate) mod page;

use async_trait::async_trait;
use serde_json::{Value, json};

use everruns_contracts::runtime::browser_use::{
    BROWSER_TOOL_NAME, BROWSER_USE_CAPABILITY_ID, BROWSER_USE_KV_PREFIX, BROWSER_USE_SYSTEM_PROMPT,
    BrowserAction, BrowserCall, BrowserUseConfig, DEFAULT_BROWSER_SCROLL, MAX_STATE_TABS, Target,
    browser_tool_schema, clean_state_text, key_sequence,
};
use everruns_contracts::runtime::computer_use::{ComputerAction, DisplaySize, Screenshot};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::tool_types::{ToolHints, ToolResultImage};
use tracing::warn;

use crate::browserless::browser_egress::BrowserEgress;
use crate::browserless::cdp::{CdpSession, DEFAULT_RECONNECT_TIMEOUT_MS, PageTarget};
use crate::browserless::computer::{CdpDisplay, is_local_page, url_blocked};
use crate::browserless::session_tools::{register_session_endpoint, try_get_cdp_session};
use crate::browserless::state::get_api_token;

use page::RefTable;

const ACTION_COUNT_KEY: &str = "browser_use.action_count";
const ACTIVE_TAB_KEY: &str = "browser_use.active_tab";
const KNOWN_TABS_KEY: &str = "browser_use.tabs";
const CURSOR_KEY: &str = "browser_use.cursor";
const REFS_KEY_PREFIX: &str = "browser_use.refs.";

const DESCRIPTION: &str = "Operate a web browser. Read the page structure with read_page or find \
(elements come with refs such as ref_3), act on elements by ref or on viewport coordinates from a \
screenshot, fill forms with form_input, and manage tabs. Every result reports the open tabs.";

/// The `browser` tool on a Browserless browser.
pub struct BrowserTool {
    config: BrowserUseConfig,
}

impl BrowserTool {
    pub fn new(config: BrowserUseConfig) -> Self {
        Self { config }
    }

    async fn charge(&self, context: &ToolContext) -> Result<u32, ToolExecutionResult> {
        let used = load(context, ACTION_COUNT_KEY)
            .await
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        if used >= self.config.max_actions_per_session {
            return Err(ToolExecutionResult::tool_error(format!(
                "Browser action budget exhausted: this session used all {} actions. Stop using \
                 the browser and report what you have.",
                self.config.max_actions_per_session
            )));
        }
        save(context, ACTION_COUNT_KEY, &(used + 1).to_string()).await;
        Ok(used + 1)
    }

    async fn run(&self, call: BrowserCall, context: &ToolContext) -> ToolExecutionResult {
        let viewport = self.config.viewport();
        if let Err(e) = call.validate(viewport) {
            return ToolExecutionResult::tool_error(e);
        }
        if let BrowserAction::Navigate { url } = &call.action
            && let Err(e) = navigation_target(url).and_then(|url| match url {
                Navigation::Url(url) => url_blocked(context, &url).map_or(Ok(()), Err),
                Navigation::History(_) => Ok(()),
            })
        {
            return ToolExecutionResult::tool_error(e);
        }
        let used = match self.charge(context).await {
            Ok(used) => used,
            Err(result) => return result,
        };
        let mut session = match connect(context).await {
            Ok(session) => session,
            Err(result) => return result,
        };
        let outcome = self.drive(&call, context, &mut session, viewport).await;
        let state = match &outcome {
            Ok(_) => Some(browser_state(context, &mut session.display).await),
            Err(_) => None,
        };
        session.release(context).await;
        let (done, state) = match (outcome, state) {
            (Ok(done), Some(Ok(state))) => (done, state),
            (Ok(_), Some(Err(e))) => {
                return ToolExecutionResult::tool_error(format!(
                    "{} ran, but reading the open tabs failed: {e}",
                    call.action.name()
                ));
            }
            (Err(e), _) => return ToolExecutionResult::tool_error(e),
            (Ok(_), None) => unreachable!("the state is read after every success"),
        };
        {
            {
                let mut result = json!({
                    "status": "ok",
                    "action": call.action.name(),
                    "text": done.text,
                    "browser_state": state,
                    "actions_used": used,
                    "actions_limit": self.config.max_actions_per_session,
                });
                if let Some(tab) = done.tab {
                    result["tab_id"] = json!(tab);
                }
                match done.image {
                    Some(shot) => ToolExecutionResult::success_with_images(
                        result,
                        vec![ToolResultImage {
                            base64: shot.base64,
                            media_type: shot.media_type,
                        }],
                    ),
                    None => ToolExecutionResult::Success(result),
                }
            }
        }
    }

    /// Select the tab, run the action, and guard where the page ended up.
    async fn drive(
        &self,
        call: &BrowserCall,
        context: &ToolContext,
        session: &mut Connected,
        viewport: DisplaySize,
    ) -> Result<Done, String> {
        let tab = self.select_tab(call, context, session).await?;
        session.display.resize(viewport).await?;
        let done = match &call.action {
            BrowserAction::NewTab
            | BrowserAction::ListTabs
            | BrowserAction::SwitchTab
            | BrowserAction::CloseTab => tab,
            action => {
                let done = act(action, context, &mut session.display, viewport).await;
                // THREAT[TM-TOOL-015]: the page may have navigated itself.
                if let Ok(url) = session.display.current_url().await
                    && !is_local_page(&url)
                    && let Some(reason) = url_blocked(context, &url)
                {
                    let _ = session.display.session_mut().navigate("about:blank").await;
                    return Err(format!(
                        "the page navigated to a blocked address and was reset to a blank page ({reason})"
                    ));
                }
                done?
            }
        };
        save(
            context,
            ACTIVE_TAB_KEY,
            session.display.session_mut().page_target_id(),
        )
        .await;
        Ok(done)
    }

    /// Attach the tab the call acts on; tab members do their work here.
    async fn select_tab(
        &self,
        call: &BrowserCall,
        context: &ToolContext,
        session: &mut Connected,
    ) -> Result<Done, String> {
        let cdp = session.display.session_mut();
        match &call.action {
            BrowserAction::NewTab => {
                let id = cdp.open_page().await?;
                return Ok(Done::text(format!(
                    "Created new tab with tab_id: {id}, URL: about:blank. It is now the current tab."
                ))
                .on(id));
            }
            BrowserAction::SwitchTab => {
                let id = call.tab_id.clone().unwrap_or_default();
                cdp.attach_page(&id).await?;
                return Ok(Done::text(format!("Switched to tab {id}")).on(id));
            }
            BrowserAction::CloseTab => {
                let id = call.tab_id.clone().unwrap_or_default();
                cdp.close_page(&id).await?;
                let remaining: Vec<PageTarget> = cdp
                    .list_pages()
                    .await?
                    .into_iter()
                    .filter(|page| page.target_id != id)
                    .collect();
                // Keep one tab so the next call has a page to act on.
                match remaining.first() {
                    Some(page) if cdp.page_target_id().is_empty() => {
                        cdp.attach_page(&page.target_id).await?
                    }
                    Some(_) => {}
                    None => {
                        cdp.open_page().await?;
                    }
                }
                return Ok(Done::text(format!("Closed tab {id}")));
            }
            _ => {}
        }
        let wanted = match &call.tab_id {
            Some(id) => Some(id.clone()),
            None => load(context, ACTIVE_TAB_KEY).await,
        };
        // A remembered tab that closed is skipped: the call stays on the
        // attached page. A tab the call names must exist.
        if let Some(id) = wanted
            && id != cdp.page_target_id()
            && let Err(e) = cdp.attach_page(&id).await
            && call.tab_id.is_some()
        {
            return Err(e);
        }
        if matches!(call.action, BrowserAction::ListTabs) {
            let pages = cdp.list_pages().await?;
            let lines: Vec<String> = pages
                .iter()
                .map(|page| {
                    format!(
                        "{}: {} ({})",
                        page.target_id,
                        clean_state_text(&page.title),
                        clean_state_text(&page.url)
                    )
                })
                .collect();
            return Ok(Done::text(if lines.is_empty() {
                "No tabs available".to_string()
            } else {
                format!("Available tabs:\n{}", lines.join("\n"))
            }));
        }
        Ok(Done::text(String::new()))
    }
}

/// What an action produced.
struct Done {
    text: String,
    image: Option<Screenshot>,
    tab: Option<String>,
}

impl Done {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            image: None,
            tab: None,
        }
    }

    fn image(shot: Screenshot) -> Self {
        Self {
            text: String::new(),
            image: Some(shot),
            tab: None,
        }
    }

    fn on(mut self, tab: String) -> Self {
        self.tab = Some(tab);
        self
    }
}

enum Navigation {
    Url(String),
    History(&'static str),
}

/// What `navigate` loads: a history step, or an http(s) URL (https when the
/// address has no scheme).
fn navigation_target(url: &str) -> Result<Navigation, String> {
    let url = url.trim();
    match url {
        "back" => return Ok(Navigation::History("back")),
        "forward" => return Ok(Navigation::History("forward")),
        "reload" => return Ok(Navigation::History("reload")),
        _ => {}
    }
    let has_scheme = url
        .split_once(':')
        .is_some_and(|(scheme, _)| {
            !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        })
        // `example.com:8080/path` is a host and port, not a scheme.
        && !url
            .split_once(':')
            .is_some_and(|(_, rest)| rest.starts_with(|c: char| c.is_ascii_digit()));
    let full = if has_scheme {
        url.to_string()
    } else {
        format!("https://{url}")
    };
    let lower = full.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        Ok(Navigation::Url(full))
    } else {
        Err("Navigation refused. Only http and https URLs are allowed.".to_string())
    }
}

/// Run one non-tab action on the attached page.
async fn act(
    action: &BrowserAction,
    context: &ToolContext,
    display: &mut CdpDisplay,
    viewport: DisplaySize,
) -> Result<Done, String> {
    if let Some(target) = click_target(action) {
        let point = resolve(context, display, target, viewport).await?;
        display.perform(&click_at(action, point)).await?;
        settle().await;
        return Ok(Done::text("Clicked."));
    }
    match action {
        BrowserAction::Navigate { url } => {
            let session = display.session_mut();
            match navigation_target(url)? {
                Navigation::History("reload") => {
                    session.send_command("Page.reload", json!({})).await?;
                    wait_for_load(session).await;
                }
                Navigation::History(step) => {
                    let history = session
                        .send_command("Page.getNavigationHistory", json!({}))
                        .await?;
                    let current = history
                        .get("currentIndex")
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    let index = if step == "back" {
                        current - 1
                    } else {
                        current + 1
                    };
                    let entry = history
                        .get("entries")
                        .and_then(Value::as_array)
                        .and_then(|entries| {
                            usize::try_from(index).ok().and_then(|i| entries.get(i))
                        })
                        .and_then(|entry| entry.get("id"))
                        .cloned()
                        .ok_or_else(|| format!("there is no page to go {step} to"))?;
                    session
                        .send_command("Page.navigateToHistoryEntry", json!({ "entryId": entry }))
                        .await?;
                    wait_for_load(session).await;
                }
                Navigation::Url(url) => {
                    session.navigate(&url).await?;
                }
            }
            let url = session.get_url().await.unwrap_or_default();
            let title = session.get_title().await.unwrap_or_default();
            Ok(Done::text(format!(
                "Navigated to {} ({})",
                clean_state_text(&url),
                clean_state_text(&title)
            )))
        }
        BrowserAction::Screenshot => Ok(Done::image(display.screenshot().await?)),
        BrowserAction::Zoom { region } => Ok(Done::image(display.zoom(*region).await?)),
        BrowserAction::Hover { target } | BrowserAction::MouseMove { target } => {
            let point = resolve(context, display, target, viewport).await?;
            display
                .perform(&ComputerAction::MouseMove { coordinate: point })
                .await?;
            Ok(Done::text(
                if matches!(action, BrowserAction::Hover { .. }) {
                    "Hovered."
                } else {
                    "Moved the pointer."
                },
            ))
        }
        BrowserAction::LeftClickDrag { from, target } => {
            let start = resolve(context, display, from, viewport).await?;
            let end = resolve(context, display, target, viewport).await?;
            display
                .perform(&ComputerAction::LeftClickDrag {
                    start_coordinate: start,
                    coordinate: end,
                    text: None,
                })
                .await?;
            Ok(Done::text("Dragged."))
        }
        BrowserAction::LeftMouseDown { target } | BrowserAction::LeftMouseUp { target } => {
            let point = resolve(context, display, target, viewport).await?;
            display
                .perform(&ComputerAction::MouseMove { coordinate: point })
                .await?;
            let down = matches!(action, BrowserAction::LeftMouseDown { .. });
            display
                .perform(&if down {
                    ComputerAction::LeftMouseDown
                } else {
                    ComputerAction::LeftMouseUp
                })
                .await?;
            Ok(Done::text(if down {
                "Pressed the left button."
            } else {
                "Released the left button."
            }))
        }
        BrowserAction::Scroll {
            target,
            scroll_direction,
            scroll_amount,
        } => {
            let point = resolve(context, display, target, viewport).await?;
            display
                .perform(&ComputerAction::Scroll {
                    coordinate: Some(point),
                    scroll_direction: *scroll_direction,
                    scroll_amount: scroll_amount.unwrap_or(DEFAULT_BROWSER_SCROLL),
                })
                .await?;
            Ok(Done::text("Scrolled."))
        }
        BrowserAction::ScrollTo { target } => {
            let Target::Ref { reference } = target else {
                return Err("scroll_to takes a ref target".to_string());
            };
            let (backend, _) = ref_node(context, display, reference).await?;
            page::scroll_into_view(display.session_mut(), backend, reference).await?;
            Ok(Done::text(format!("Scrolled {reference} into view.")))
        }
        BrowserAction::Type { text } => {
            display
                .perform(&ComputerAction::Type { text: text.clone() })
                .await?;
            Ok(Done::text("Typed."))
        }
        BrowserAction::Key { text, repeat } => {
            let combos = key_sequence(text)?;
            for _ in 0..repeat.unwrap_or(1) {
                for combo in &combos {
                    display
                        .perform(&ComputerAction::Key {
                            text: combo.clone(),
                            repeat: None,
                        })
                        .await?;
                }
            }
            settle().await;
            Ok(Done::text(format!("Pressed {text}.")))
        }
        BrowserAction::HoldKey { text, duration } => {
            display
                .perform(&ComputerAction::HoldKey {
                    text: text.clone(),
                    duration: *duration,
                })
                .await?;
            Ok(Done::text(format!("Held {text} for {duration} s.")))
        }
        BrowserAction::Wait { duration } => {
            display
                .perform(&ComputerAction::Wait {
                    duration: *duration,
                })
                .await?;
            Ok(Done::text(format!("Waited {duration} s.")))
        }
        BrowserAction::ReadPage {
            filter,
            depth,
            reference,
        } => {
            let tab = display.session_mut().page_target_id().to_string();
            let (_, document) = page::main_frame(display.session_mut()).await?;
            let mut table = load_refs(context, &tab).await.for_document(&document);
            let start = match reference {
                Some(reference) => Some(page::backend_for(&table, &document, reference)?),
                None => None,
            };
            let tree = page::capture(display.session_mut(), viewport).await?;
            let text = tree.read(*filter, *depth, start, &mut table)?;
            save_refs(context, &tab, &table).await;
            Ok(Done::text(text))
        }
        BrowserAction::Find { query } => {
            let tab = display.session_mut().page_target_id().to_string();
            let (_, document) = page::main_frame(display.session_mut()).await?;
            let mut table = load_refs(context, &tab).await.for_document(&document);
            let tree = page::capture(display.session_mut(), viewport).await?;
            let text = tree.find(query, &mut table);
            save_refs(context, &tab, &table).await;
            Ok(Done::text(text))
        }
        BrowserAction::GetPageText => Ok(Done::text(page::page_text(display.session_mut()).await?)),
        BrowserAction::FormInput { target, value } => {
            let Target::Ref { reference } = target else {
                return Err("form_input takes a ref target".to_string());
            };
            let (backend, _) = ref_node(context, display, reference).await?;
            let text = page::set_value(display.session_mut(), backend, reference, value).await?;
            Ok(Done::text(text))
        }
        // Tab members and clicks are handled before this point.
        _ => Ok(Done::text(String::new())),
    }
}

/// The target of a click member.
fn click_target(action: &BrowserAction) -> Option<&Target> {
    match action {
        BrowserAction::LeftClick { target, .. }
        | BrowserAction::RightClick { target, .. }
        | BrowserAction::MiddleClick { target, .. }
        | BrowserAction::DoubleClick { target, .. }
        | BrowserAction::TripleClick { target, .. } => Some(target),
        _ => None,
    }
}

/// The `computer` click a click member performs at `point`.
fn click_at(action: &BrowserAction, point: [u32; 2]) -> ComputerAction {
    let coordinate = Some(point);
    match action.clone() {
        BrowserAction::RightClick { modifiers, .. } => ComputerAction::RightClick {
            coordinate,
            text: modifiers,
        },
        BrowserAction::MiddleClick { modifiers, .. } => ComputerAction::MiddleClick {
            coordinate,
            text: modifiers,
        },
        BrowserAction::DoubleClick { modifiers, .. } => ComputerAction::DoubleClick {
            coordinate,
            text: modifiers,
        },
        BrowserAction::TripleClick { modifiers, .. } => ComputerAction::TripleClick {
            coordinate,
            text: modifiers,
        },
        BrowserAction::LeftClick { modifiers, .. } => ComputerAction::LeftClick {
            coordinate,
            text: modifiers,
        },
        _ => ComputerAction::LeftClick {
            coordinate,
            text: None,
        },
    }
}

/// Give the page a moment to react before the next call reads it.
async fn settle() {
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

async fn wait_for_load(session: &mut CdpSession) {
    let _ = session
        .send_command(
            "Runtime.evaluate",
            json!({
                "expression": "new Promise(r => { if (document.readyState === 'complete') r(); else window.addEventListener('load', r); })",
                "awaitPromise": true,
                "timeout": 30000
            }),
        )
        .await;
}

/// The backend node a ref names on the attached tab's current document.
async fn ref_node(
    context: &ToolContext,
    display: &mut CdpDisplay,
    reference: &str,
) -> Result<(i64, String), String> {
    let tab = display.session_mut().page_target_id().to_string();
    let (_, document) = page::main_frame(display.session_mut()).await?;
    let table = load_refs(context, &tab).await;
    page::backend_for(&table, &document, reference).map(|backend| (backend, document))
}

/// Viewport point of a target: the coordinate, or the ref's element center.
async fn resolve(
    context: &ToolContext,
    display: &mut CdpDisplay,
    target: &Target,
    viewport: DisplaySize,
) -> Result<[u32; 2], String> {
    match target {
        Target::Coordinate { x, y } => Ok([*x, *y]),
        Target::Ref { reference } => {
            let (backend, _) = ref_node(context, display, reference).await?;
            page::node_center(display.session_mut(), backend, reference, viewport).await
        }
    }
}

/// The state report: every tab of the context, and tabs opened since the
/// last report.
async fn browser_state(context: &ToolContext, display: &mut CdpDisplay) -> Result<Value, String> {
    let session = display.session_mut();
    let active = session.page_target_id().to_string();
    let pages: Vec<PageTarget> = session.list_pages().await?;
    let known: Vec<String> = load(context, KNOWN_TABS_KEY)
        .await
        .map(|value| value.split(',').map(str::to_string).collect())
        .unwrap_or_default();
    let tabs: Vec<Value> = pages
        .iter()
        .take(MAX_STATE_TABS)
        .map(|page| {
            let mut tab = json!({
                "tab_id": page.target_id,
                "title": clean_state_text(&page.title),
                "url": clean_state_text(&page.url),
            });
            if page.target_id == active {
                tab["active"] = json!(true);
            }
            tab
        })
        .collect();
    let opened: Vec<Value> = pages
        .iter()
        .take(MAX_STATE_TABS)
        .filter(|page| !known.contains(&page.target_id))
        .map(|page| json!({ "type": "tab_opened", "tab_id": page.target_id }))
        .collect();
    let ids: Vec<&str> = pages.iter().map(|page| page.target_id.as_str()).collect();
    save(context, KNOWN_TABS_KEY, &ids.join(",")).await;
    let mut state = json!({ "tabs": tabs });
    // The first report lists every tab; only later ones name new tabs.
    if !known.is_empty() && !opened.is_empty() {
        state["state_changes"] = json!(opened);
    }
    Ok(state)
}

// ============================================================================
// Session storage
// ============================================================================

async fn load(context: &ToolContext, key: &str) -> Option<String> {
    debug_assert!(key.starts_with(BROWSER_USE_KV_PREFIX));
    let storage = context.storage_store.as_ref()?;
    storage
        .get_value(context.session_id, key)
        .await
        .ok()
        .flatten()
        .filter(|value| !value.is_empty())
}

async fn save(context: &ToolContext, key: &str, value: &str) {
    debug_assert!(key.starts_with(BROWSER_USE_KV_PREFIX));
    if let Some(storage) = context.storage_store.as_ref()
        && let Err(e) = storage.set_value(context.session_id, key, value).await
    {
        warn!("browser_use: failed to save {key}: {e}");
    }
}

async fn load_refs(context: &ToolContext, tab: &str) -> RefTable {
    load(context, &format!("{REFS_KEY_PREFIX}{tab}"))
        .await
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

async fn save_refs(context: &ToolContext, tab: &str, table: &RefTable) {
    if let Ok(value) = serde_json::to_string(table) {
        save(context, &format!("{REFS_KEY_PREFIX}{tab}"), &value).await;
    }
}

// ============================================================================
// Connection
// ============================================================================

struct Connected {
    display: CdpDisplay,
}

/// Connect to the session's persistent browser, opening one if needed.
async fn connect(context: &ToolContext) -> Result<Connected, ToolExecutionResult> {
    let api_token = get_api_token(context).await?;
    let session = match try_get_cdp_session(context).await {
        Some(session) => session,
        None => {
            let ws_url = crate::browserless::browser_session_url(
                &crate::browserless::browserless_ws_base(),
                &api_token,
            );
            CdpSession::connect(&ws_url, BrowserEgress::for_context(context), None)
                .await
                .map_err(ToolExecutionResult::tool_error)?
        }
    };
    Connected::attach(session, context).await
}

impl Connected {
    async fn attach(
        session: CdpSession,
        context: &ToolContext,
    ) -> Result<Self, ToolExecutionResult> {
        let (cursor, held) = load(context, CURSOR_KEY)
            .await
            .and_then(|value| crate::browserless::computer::parse_cursor(&value))
            .unwrap_or(([0, 0], false));
        // The viewport is sized per tab in `drive`, after the tab is chosen.
        let mut display = CdpDisplay::detached(session, cursor);
        display.set_button_held(held);
        Ok(Self { display })
    }

    /// Keep the browser alive for the next call and remember the cursor.
    async fn release(self, context: &ToolContext) {
        let cursor = self.display.cursor();
        let held = self.display.button_held();
        let value = if held {
            format!("{},{},down", cursor[0], cursor[1])
        } else {
            format!("{},{}", cursor[0], cursor[1])
        };
        save(context, CURSOR_KEY, &value).await;
        let mut session = self.display.into_session();
        match session.reconnect(DEFAULT_RECONNECT_TIMEOUT_MS).await {
            Ok(endpoint) => {
                let browser_context_id = session.browser_context_id().to_string();
                if let Err(e) =
                    register_session_endpoint(context, endpoint, &browser_context_id).await
                {
                    warn!("browser_use: failed to persist the browser session: {e:?}");
                }
            }
            Err(e) => warn!("browser_use: failed to keep the browser alive: {e}"),
        }
        session.disconnect().await;
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        BROWSER_TOOL_NAME
    }

    fn display_name(&self) -> Option<&str> {
        Some("Browser")
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn parameters_schema(&self) -> Value {
        browser_tool_schema(self.config.viewport())
    }

    fn hints(&self) -> ToolHints {
        // One browser per session: actions must apply in order.
        ToolHints::default()
            .with_open_world(true)
            .with_long_running(true)
            .with_concurrency_class(BROWSER_TOOL_NAME)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("browser requires a session context")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        match BrowserCall::from_arguments(&arguments) {
            Ok(call) => self.run(call, context).await,
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Capability
// ============================================================================

/// Browser use on a Browserless browser.
pub struct BrowserUseCapability;

impl everruns_contracts::runtime::capabilities::Capability for BrowserUseCapability {
    fn id(&self) -> &str {
        BROWSER_USE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Browser Use"
    }

    fn description(&self) -> &str {
        "Let the agent work in a browser through the page structure: it reads pages as an \
         accessibility tree with element refs, clicks and fills forms by ref or by screen \
         coordinates, and opens and switches tabs. Works with any model; screenshots need one \
         that accepts images. Pages can try to steer the agent; keep credentials out of reach. \
         Runs on a Browserless browser."
    }

    fn status(&self) -> everruns_contracts::runtime::capabilities::CapabilityStatus {
        everruns_contracts::runtime::capabilities::CapabilityStatus::Available
    }

    fn risk_level(&self) -> everruns_contracts::runtime::capabilities::RiskLevel {
        everruns_contracts::runtime::capabilities::RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("globe")
    }

    fn category(&self) -> Option<&str> {
        Some("Browser")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(BROWSER_USE_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        self.tools_with_config(&Value::Null)
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        vec![Box::new(BrowserTool::new(
            BrowserUseConfig::from_value_or_default(config),
        ))]
    }

    fn config_schema(&self) -> Option<Value> {
        Some(BrowserUseConfig::json_schema())
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        BrowserUseConfig::from_value(config).map(|_| ())
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec![everruns_contracts::runtime::LEASED_RESOURCES_FEATURE]
    }
}

/// Keep the shared helpers referenced from one place for tests.
#[cfg(test)]
pub(crate) fn test_navigation_target(url: &str) -> Result<String, String> {
    navigation_target(url).map(|target| match target {
        Navigation::Url(url) => url,
        Navigation::History(step) => step.to_string(),
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
