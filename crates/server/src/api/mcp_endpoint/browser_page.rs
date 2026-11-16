//! Human-readable page for a browser that opens `/mcp`.
//!
//! Decision: `/mcp` speaks JSON-RPC over POST, so a person who pastes the URL
//! into a browser used to get a bare 405. A GET whose `Accept` asks for HTML
//! (and not for an SSE stream) now gets a static page that says the URL is for
//! MCP clients and how to connect one. Every other GET still gets the 405 the
//! Streamable HTTP transport expects from a server that offers no GET stream,
//! so MCP clients and OAuth discovery see no change.
//!
//! The page carries no user data and needs no auth. Its only script is the
//! copy button; the page sets its own CSP that allows exactly that script by
//! hash, since the baseline CSP forbids inline scripts.

use super::AppState;
use super::cards::escape_html;
use super::discovery::MCP_ICON_PATH;
use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

const SETUP_GUIDE_URL: &str = "https://everruns.com/docs/getting-started/use-in-ai-tools/";

const COPY_SCRIPT: &str = r#"document.querySelectorAll("[data-copy]").forEach(function(b){b.addEventListener("click",function(){var t=document.getElementById(b.dataset.copy).textContent;navigator.clipboard.writeText(t).then(function(){b.textContent="Copied";setTimeout(function(){b.textContent="Copy"},1500)})})});"#;

static PAGE_CSP: LazyLock<HeaderValue> = LazyLock::new(|| {
    let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(COPY_SCRIPT));
    let csp = format!(
        "default-src 'none'; script-src 'sha256-{hash}'; style-src 'unsafe-inline'; \
         img-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"
    );
    HeaderValue::from_str(&csp).expect("CSP is ASCII")
});

/// True when the request is a browser navigation: it accepts HTML and does
/// not ask for the SSE stream an MCP client would open with GET.
fn wants_html(headers: &HeaderMap) -> bool {
    let accept = headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect::<Vec<_>>()
        .join(",")
        .to_ascii_lowercase();
    accept.contains("text/html") && !accept.contains("text/event-stream")
}

pub(super) async fn handle_get(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !wants_html(&headers) {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, HeaderValue::from_static("POST"))],
        )
            .into_response();
    }
    let url = state.mcp_resource.as_deref().unwrap_or("/mcp");
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (header::CONTENT_SECURITY_POLICY, PAGE_CSP.clone()),
            (header::VARY, HeaderValue::from_static("Accept")),
        ],
        render_page(url),
    )
        .into_response()
}

const STYLE: &str = r#"
:root{--background:0 0% 98%;--foreground:0 0% 10%;--card:0 0% 100%;--muted:0 0% 96%;--muted-foreground:0 0% 42%;--primary:220 62% 13%;--primary-foreground:0 0% 98%;--accent:43 60% 53%;--border:0 0% 88%;--dots:220 62% 13% / .035;color-scheme:light dark}
@media (prefers-color-scheme:dark){:root{--background:220 15% 8%;--foreground:0 0% 95%;--card:220 15% 10%;--muted:220 15% 15%;--muted-foreground:0 0% 65%;--primary:0 0% 95%;--primary-foreground:220 15% 8%;--border:220 15% 18%;--dots:43 60% 53% / .045}}
*{box-sizing:border-box}
body{margin:0;min-height:100vh;font:14px/1.5 Geist,ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif;letter-spacing:-.01em;color:hsl(var(--foreground));background-color:hsl(var(--background));background-image:radial-gradient(hsl(var(--dots)) 1px,transparent 1px);background-size:24px 24px;padding:48px 24px}
main{max-width:640px;margin:0 auto}
.brand{display:flex;width:max-content;align-items:center;gap:8px;font-weight:600;font-size:15px;margin-bottom:48px;color:inherit;text-decoration:none}
.brand img{width:24px;height:24px}
.badge{display:inline-flex;align-items:center;gap:6px;font-size:12px;font-weight:500;padding:2px 8px;border:1px solid hsl(var(--border));background:hsl(var(--card));color:hsl(var(--muted-foreground))}
.badge i{width:6px;height:6px;border-radius:50%;background:hsl(var(--accent))}
h1{font-size:30px;font-weight:600;letter-spacing:-.02em;line-height:1.2;margin:16px 0 12px}
.lead{font-size:16px;line-height:1.6;color:hsl(var(--muted-foreground));margin:0 0 32px}
.card{background:hsl(var(--card));border:1px solid hsl(var(--border))}
.card+.card{margin-top:24px}
.card h2{font-size:13px;font-weight:600;margin:0;padding:12px 16px;border-bottom:1px solid hsl(var(--border))}
.url{display:flex;align-items:center;gap:8px;padding:16px}
.url code{flex:1;min-width:0;overflow-wrap:anywhere;font:14px ui-monospace,"Geist Mono",SFMono-Regular,Menlo,monospace}
button{font:inherit;font-size:12px;font-weight:500;height:32px;padding:0 12px;border:1px solid hsl(var(--primary));background:hsl(var(--primary));color:hsl(var(--primary-foreground));cursor:pointer}
dl{margin:0;display:grid;grid-template-columns:120px 1fr;border-top:1px solid hsl(var(--border))}
dt,dd{margin:0;padding:12px 16px;border-bottom:1px solid hsl(var(--border))}
dl>:nth-last-child(-n+2){border-bottom:0}
dt{color:hsl(var(--muted-foreground));font-size:13px;font-weight:500}
dd code{font:13px ui-monospace,"Geist Mono",SFMono-Regular,Menlo,monospace}
ul{list-style:none;margin:0;padding:0}
li{padding:12px 16px;border-bottom:1px solid hsl(var(--border))}
li:last-child{border-bottom:0}
li b{display:block;font-weight:600;margin-bottom:4px}
pre{margin:0;padding:8px 12px;background:hsl(var(--muted));overflow-x:auto;font:13px/1.5 ui-monospace,"Geist Mono",SFMono-Regular,Menlo,monospace}
a{color:inherit;text-underline-offset:3px}
.more{display:block;padding:12px 16px;border-top:1px solid hsl(var(--border));font-weight:500}
footer{margin-top:32px;font-size:13px;color:hsl(var(--muted-foreground))}
@media (max-width:560px){body{padding:24px 16px}.brand{margin-bottom:32px}h1{font-size:25px}dl{grid-template-columns:1fr}dt{border-bottom:0;padding-bottom:0}}
"#;

fn render_page(url: &str) -> String {
    let url = escape_html(url);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex">
<title>Everruns MCP endpoint</title>
<style>{STYLE}</style>
</head>
<body>
<main>
  <a class="brand" href="/"><img src="{MCP_ICON_PATH}" alt="">Everruns</a>
  <span class="badge"><i></i>MCP endpoint · for agents</span>
  <h1>This URL is for your agent, not your browser</h1>
  <p class="lead">This is the Everruns MCP server. Coding agents and other MCP clients connect here to run your agents, open sessions and use your tools. There is nothing to see in a browser: add the URL below to your client instead.</p>

  <section class="card">
    <h2>Server URL</h2>
    <div class="url"><code id="mcp-url">{url}</code><button type="button" data-copy="mcp-url">Copy</button></div>
    <dl>
      <dt>Transport</dt><dd>Streamable HTTP (JSON-RPC over POST)</dd>
      <dt>Sign-in</dt><dd>OAuth. Your client registers itself and opens a browser for you to approve it, so there is no key to paste. A personal access token sent as <code>Authorization: Bearer</code> also works.</dd>
      <dt>Access</dt><dd>Approved clients are listed in <a href="/settings/ai-clients">Settings → AI clients</a>, where you can revoke them.</dd>
    </dl>
  </section>

  <section class="card">
    <h2>Connect a client</h2>
    <ul>
      <li><b>Claude Code</b><pre>claude mcp add --transport http everruns {url}</pre></li>
      <li><b>Codex</b><pre>codex mcp add everruns --url {url}</pre></li>
      <li><b>Cursor, VS Code and other clients</b>Add a remote (HTTP) MCP server with the URL above.</li>
    </ul>
    <a class="more" href="{SETUP_GUIDE_URL}">Full setup guide, including the Everruns plugin with skills →</a>
  </section>

  <footer>Looking for the app? <a href="/">Open Everruns</a></footer>
</main>
<script>{COPY_SCRIPT}</script>
</body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accept(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn browser_navigation_wants_html() {
        assert!(wants_html(&accept(
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
        )));
    }

    #[test]
    fn mcp_clients_do_not_get_html() {
        assert!(!wants_html(&accept("text/event-stream")));
        assert!(!wants_html(&accept("application/json, text/event-stream")));
        assert!(!wants_html(&accept("text/html, text/event-stream")));
        assert!(!wants_html(&accept("*/*")));
        assert!(!wants_html(&HeaderMap::new()));
    }

    #[test]
    fn page_shows_the_escaped_endpoint_url() {
        let html = render_page("https://example.com/mcp?a=<b>");
        assert!(html.contains("https://example.com/mcp?a=&lt;b&gt;"));
        assert!(!html.contains("<b>\""));
        assert!(html.contains("This URL is for your agent"));
        assert!(html.contains(SETUP_GUIDE_URL));
    }

    #[test]
    fn csp_allows_exactly_the_copy_script() {
        let csp = PAGE_CSP.to_str().unwrap();
        let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(COPY_SCRIPT));
        assert!(csp.contains(&format!("script-src 'sha256-{hash}'")));
        assert!(render_page("/mcp").contains(&format!("<script>{COPY_SCRIPT}</script>")));
    }
}
