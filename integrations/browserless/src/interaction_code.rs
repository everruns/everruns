//! Puppeteer function source for Browserless `/function` calls.
//!
//! The REST screenshot, content, and scrape APIs honor `rejectRequestPattern`.
//! The function API does not, so interact and navigate embed the same patterns
//! and abort the request before it is continued.

use serde_json::Value;
use tracing::debug;

use crate::state::{build_cookie_extraction_code, build_cookie_injection_code};

const MAX_WAIT_MS: u64 = 120_000;

/// Cap a wait/timeout value to MAX_WAIT_MS. // THREAT[TM-TOOL-016]
fn cap_wait_ms(ms: u64) -> u64 {
    ms.min(MAX_WAIT_MS)
}

/// Puppeteer request interception installed before any `page.goto`.
pub(crate) fn request_interception_prelude(patterns: &[String]) -> String {
    if patterns.is_empty() {
        return String::new();
    }
    let patterns_js = serde_json::to_string(patterns).unwrap_or_else(|_| "[]".to_string());
    format!(
        "  const networkRejectPatterns = {patterns_js};\n\
         \x20 await page.setRequestInterception(true);\n\
         \x20 page.on('request', (req) => {{\n\
         \x20   const url = req.url();\n\
         \x20   let blocked = false;\n\
         \x20   for (const pattern of networkRejectPatterns) {{\n\
         \x20     try {{\n\
         \x20       if (url.match(pattern)) {{ blocked = true; break; }}\n\
         \x20     }} catch {{ blocked = true; break; }}\n\
         \x20   }}\n\
         \x20   if (blocked) req.abort('blockedbyclient');\n\
         \x20   else req.continue();\n\
         \x20 }});\n"
    )
}

/// Build Puppeteer function code with no session rejection patterns.
#[cfg(test)]
pub(crate) fn build_interaction_code(
    url: &str,
    steps: &[Value],
    want_screenshot: bool,
    want_content: bool,
    cookies: &[Value],
) -> String {
    build_interaction_code_with_policy(url, steps, want_screenshot, want_content, cookies, &[])
}

/// Build Puppeteer function code from a list of interaction steps.
/// Each step is executed sequentially in a fresh browser session.
/// If `cookies` is non-empty, they are injected before navigation.
/// Cookies are always extracted after steps for persistence.
///
/// `reject_patterns` is installed before `page.goto`. An empty slice leaves
/// request interception off, which is the behavior when no session ACL is set.
pub(crate) fn build_interaction_code_with_policy(
    url: &str,
    steps: &[Value],
    want_screenshot: bool,
    want_content: bool,
    cookies: &[Value],
    reject_patterns: &[String],
) -> String {
    let mut code = String::new();
    code.push_str("export default async ({ page }) => {\n");
    code.push_str(&request_interception_prelude(reject_patterns));
    code.push_str(&build_cookie_injection_code(cookies));
    code.push_str(&format!(
        "  await page.goto({}, {{ waitUntil: 'networkidle2', timeout: 30000 }});\n",
        serde_json::to_string(url).unwrap_or_else(|_| format!("\"{}\"", url))
    ));

    for step in steps {
        let action = step
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let selector = step.get("selector").and_then(|v| v.as_str());
        let value = step.get("value").and_then(|v| v.as_str());
        let key = step.get("key").and_then(|v| v.as_str());
        let x = step.get("x").and_then(|v| v.as_f64());
        let y = step.get("y").and_then(|v| v.as_f64());
        let wait_ms = step.get("wait_ms").and_then(|v| v.as_u64());

        match action {
            "click" => {
                if let Some(sel) = selector {
                    code.push_str(&format!(
                        "  await page.waitForSelector({sel_js}, {{ timeout: 10000 }});\n\
                         \x20 await page.click({sel_js});\n",
                        sel_js = serde_json::to_string(sel).unwrap()
                    ));
                } else if let (Some(cx), Some(cy)) = (x, y) {
                    code.push_str(&format!("  await page.mouse.click({cx}, {cy});\n"));
                }
            }
            "type" => {
                if let (Some(sel), Some(text)) = (selector, value) {
                    code.push_str(&format!(
                        "  await page.waitForSelector({sel_js}, {{ timeout: 10000 }});\n\
                         \x20 await page.type({sel_js}, {val_js});\n",
                        sel_js = serde_json::to_string(sel).unwrap(),
                        val_js = serde_json::to_string(text).unwrap()
                    ));
                }
            }
            "keyboard" => {
                if let Some(k) = key {
                    code.push_str(&format!(
                        "  await page.keyboard.press({key_js});\n",
                        key_js = serde_json::to_string(k).unwrap()
                    ));
                }
            }
            "mouse_move" => {
                if let (Some(mx), Some(my)) = (x, y) {
                    code.push_str(&format!("  await page.mouse.move({mx}, {my});\n"));
                }
            }
            "touch" => {
                if let Some(sel) = selector {
                    code.push_str(&format!(
                        "  await page.waitForSelector({sel_js}, {{ timeout: 10000 }});\n\
                         \x20 await page.tap({sel_js});\n",
                        sel_js = serde_json::to_string(sel).unwrap()
                    ));
                }
            }
            "scroll" => {
                let scroll_y = step.get("value").and_then(|v| v.as_i64()).unwrap_or(500);
                code.push_str(&format!(
                    "  await page.evaluate(() => window.scrollBy(0, {scroll_y}));\n"
                ));
            }
            "wait" => {
                let ms = cap_wait_ms(wait_ms.unwrap_or(1000));
                code.push_str(&format!("  await new Promise(r => setTimeout(r, {ms}));\n"));
            }
            "wait_for_selector" => {
                if let Some(sel) = selector {
                    let timeout = cap_wait_ms(wait_ms.unwrap_or(10000));
                    code.push_str(&format!(
                        "  await page.waitForSelector({sel_js}, {{ timeout: {timeout} }});\n",
                        sel_js = serde_json::to_string(sel).unwrap()
                    ));
                }
            }
            "navigate" => {
                if let Some(nav_url) = value {
                    code.push_str(&format!(
                        "  await page.goto({url_js}, {{ waitUntil: 'networkidle2', timeout: 30000 }});\n",
                        url_js = serde_json::to_string(nav_url).unwrap()
                    ));
                }
            }
            other => {
                debug!("Unknown interaction action: {other}");
            }
        }

        // Optional per-step wait
        if action != "wait"
            && let Some(ms) = wait_ms
        {
            let capped = cap_wait_ms(ms);
            code.push_str(&format!(
                "  await new Promise(r => setTimeout(r, {capped}));\n"
            ));
        }
    }

    // Capture final state
    code.push_str("  const title = await page.title();\n");
    code.push_str("  const url = page.url();\n");

    if want_screenshot {
        code.push_str(
            "  const screenshot = await page.screenshot({ encoding: 'base64', fullPage: true });\n",
        );
    }
    if want_content {
        code.push_str("  const content = await page.content();\n");
    }

    // Extract cookies for persistence
    code.push_str(build_cookie_extraction_code());

    // Build return object with whichever fields were requested
    let mut fields = vec!["title", "url", "__cookies"];
    if want_screenshot {
        fields.push("screenshot");
    }
    if want_content {
        fields.push("content");
    }
    code.push_str(&format!(
        "  return {{ data: JSON.stringify({{ {} }}), type: 'application/json' }};\n",
        fields.join(", ")
    ));

    code.push_str("};\n");
    code
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::transport_reject_patterns;
    use everruns_core::network_access::NetworkAccessList;
    use serde_json::json;

    #[test]
    fn interaction_code_aborts_requests_outside_the_session_acl() {
        let patterns = transport_reject_patterns(&NetworkAccessList::allow_only(["example.com"]));
        let code = build_interaction_code_with_policy(
            "https://example.com/",
            &[json!({"action": "wait", "wait_ms": 1})],
            false,
            true,
            &[],
            &patterns,
        );
        let intercept = code.find("setRequestInterception").expect("intercept");
        let goto = code.find("page.goto").expect("goto");
        assert!(
            intercept < goto,
            "interception must be installed before navigation"
        );
        assert!(code.contains("evil") || code.contains("(?!"), "{code}");
        assert!(code.contains("blockedbyclient"));

        let open = build_interaction_code(
            "https://example.com/",
            &[json!({"action": "wait", "wait_ms": 1})],
            false,
            true,
            &[],
        );
        assert!(!open.contains("setRequestInterception"));
    }

    #[test]
    fn test_build_interaction_code_click() {
        let steps = vec![json!({
            "action": "click",
            "selector": "#submit-btn"
        })];
        let code = build_interaction_code("https://example.com", &steps, false, true, &[]);
        assert!(code.contains("page.goto"));
        assert!(code.contains("page.click"));
        assert!(code.contains("#submit-btn"));
        assert!(code.contains("page.content()"));
    }

    #[test]
    fn test_build_interaction_code_type() {
        let steps = vec![json!({
            "action": "type",
            "selector": "#username",
            "value": "admin"
        })];
        let code = build_interaction_code("https://example.com", &steps, false, true, &[]);
        assert!(code.contains("page.type"));
        assert!(code.contains("#username"));
        assert!(code.contains("admin"));
    }

    #[test]
    fn test_build_interaction_code_keyboard() {
        let steps = vec![json!({
            "action": "keyboard",
            "key": "Enter"
        })];
        let code = build_interaction_code("https://example.com", &steps, false, true, &[]);
        assert!(code.contains("page.keyboard.press"));
        assert!(code.contains("Enter"));
    }

    #[test]
    fn test_build_interaction_code_mouse_move() {
        let steps = vec![json!({
            "action": "mouse_move",
            "x": 100.0,
            "y": 200.0
        })];
        let code = build_interaction_code("https://example.com", &steps, false, true, &[]);
        assert!(code.contains("page.mouse.move(100, 200)"));
    }

    #[test]
    fn test_build_interaction_code_touch() {
        let steps = vec![json!({
            "action": "touch",
            "selector": ".menu-item"
        })];
        let code = build_interaction_code("https://example.com", &steps, false, true, &[]);
        assert!(code.contains("page.tap"));
    }

    #[test]
    fn test_build_interaction_code_scroll() {
        let steps = vec![json!({
            "action": "scroll",
            "value": 1000
        })];
        let code = build_interaction_code("https://example.com", &steps, false, true, &[]);
        assert!(code.contains("window.scrollBy(0, 1000)"));
    }

    #[test]
    fn test_build_interaction_code_with_screenshot() {
        let steps = vec![json!({
            "action": "click",
            "selector": "button"
        })];
        let code = build_interaction_code("https://example.com", &steps, true, false, &[]);
        assert!(code.contains("page.screenshot"));
        assert!(!code.contains("page.content()"));
    }

    #[test]
    fn test_build_interaction_code_navigate_step() {
        let steps = vec![json!({
            "action": "navigate",
            "value": "https://other.com/page"
        })];
        let code = build_interaction_code("https://example.com", &steps, false, true, &[]);
        // Should have two page.goto calls: initial + navigate step
        assert!(code.contains("https://other.com/page"));
    }

    #[test]
    fn test_build_interaction_code_wait() {
        let steps = vec![json!({
            "action": "wait",
            "wait_ms": 2000
        })];
        let code = build_interaction_code("https://example.com", &steps, false, true, &[]);
        assert!(code.contains("setTimeout(r, 2000)"));
    }

    #[test]
    fn test_build_interaction_code_wait_for_selector() {
        let steps = vec![json!({
            "action": "wait_for_selector",
            "selector": ".loaded",
            "wait_ms": 5000
        })];
        let code = build_interaction_code("https://example.com", &steps, false, true, &[]);
        assert!(code.contains("waitForSelector"));
        assert!(code.contains(".loaded"));
        assert!(code.contains("5000"));
    }

    #[test]
    fn test_build_interaction_code_click_by_coordinates() {
        let steps = vec![json!({
            "action": "click",
            "x": 50.0,
            "y": 75.0
        })];
        let code = build_interaction_code("https://example.com", &steps, false, true, &[]);
        assert!(code.contains("page.mouse.click(50, 75)"));
    }

    #[test]
    fn test_build_interaction_code_multi_step() {
        let steps = vec![
            json!({"action": "click", "selector": "#login-link"}),
            json!({"action": "wait_for_selector", "selector": "#username"}),
            json!({"action": "type", "selector": "#username", "value": "admin"}),
            json!({"action": "type", "selector": "#password", "value": "secret"}),
            json!({"action": "click", "selector": "#submit"}),
            json!({"action": "wait", "wait_ms": 2000}),
        ];
        let code = build_interaction_code("https://example.com/home", &steps, true, false, &[]);
        assert!(code.contains("#login-link"));
        assert!(code.contains("#username"));
        assert!(code.contains("#password"));
        assert!(code.contains("#submit"));
        assert!(code.contains("page.screenshot"));
    }

    #[test]
    fn test_build_interaction_code_injects_cookies() {
        let cookies = vec![json!({
            "name": "session_id",
            "value": "abc123",
            "domain": "example.com"
        })];
        let steps = vec![json!({"action": "click", "selector": "#btn"})];
        let code = build_interaction_code("https://example.com", &steps, false, true, &cookies);
        assert!(
            code.contains("setCookie"),
            "should inject cookies via setCookie: {code}"
        );
        assert!(
            code.contains("session_id"),
            "should contain cookie name: {code}"
        );
        assert!(
            code.contains("page.cookies()"),
            "should extract cookies via page.cookies(): {code}"
        );
    }

    #[test]
    fn test_build_interaction_code_no_cookies_no_injection() {
        let steps = vec![json!({"action": "click", "selector": "#btn"})];
        let code = build_interaction_code("https://example.com", &steps, false, true, &[]);
        assert!(
            !code.contains("setCookie"),
            "should not inject cookies when none provided"
        );
        // Should still extract cookies for persistence
        assert!(
            code.contains("const __cookies = await page.cookies()"),
            "should still extract cookies: {code}"
        );
    }

    #[test]
    fn test_build_interaction_code_both_screenshot_and_content() {
        let steps = vec![json!({"action": "click", "selector": "#btn"})];
        let code = build_interaction_code("https://example.com", &steps, true, true, &[]);
        assert!(
            code.contains("page.screenshot"),
            "should include screenshot capture"
        );
        assert!(
            code.contains("page.content()"),
            "should include content capture"
        );
        assert!(
            code.contains("screenshot") && code.contains("content"),
            "return object should include both fields"
        );
    }
}
