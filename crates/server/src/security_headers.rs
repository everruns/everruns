//! Browser security headers served with the UI.
//!
// Split out of `app_builder.rs` (EVE-1047): that file is pinned by the size
// ratchet, and this is the most self-contained thing in it — a pure function
// over two feature flags, with no dependency on how the app is composed.

use crate::channels;

// TM-WEB-004/005: baseline CSP stamped on every response that does not set its
// own. `frame-src 'self' data:` lets the file-preview UI embed PDFs via a
// `data:application/pdf` iframe (sandboxed viewers don't render in Chromium)
// and keeps `about:srcdoc` previews (SVG/HTML/MCP cards) working under 'self'.
// `form-action` must remain the LAST directive: the MCP OAuth consent page
// extends it by appending sources to this string (see
// `oauth_consent_page_csp`).
pub(crate) const BASE_CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; frame-src 'self' data:; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

/// CSP for the MCP OAuth consent page, whose confirm POST answers with a 302
/// to the client's callback.
///
/// Chrome checks `form-action` against every hop of a form submission's
/// redirect chain, not just the form target, and a blocked hop silently
/// cancels the whole submission: the "Authorize client" click appears to do
/// nothing. Allowing only the registered redirect origin is not enough, because
/// web callbacks routinely hop on to hosts the client never registered (Cursor
/// registers `https://www.cursor.com/...`, which 308s to `https://cursor.com/...`).
/// So any `https:` hop is allowed, plus the origin of an `http://` loopback
/// callback, the only other scheme `redirect_uri::validate_redirect_uri`
/// accepts. The page renders only escaped values and its one form posts to
/// 'self', so the wider directive gives up little.
pub(crate) fn oauth_consent_page_csp(redirect_uri: &str) -> String {
    let mut csp = format!("{BASE_CONTENT_SECURITY_POLICY} https:");
    if let Ok(url) = url::Url::parse(redirect_uri)
        && url.scheme() == "http"
    {
        csp.push(' ');
        csp.push_str(&url.origin().ascii_serialization());
    }
    csp
}

pub(crate) fn permissions_policy_header_value(
    voice_enabled: bool,
    webmcp_enabled: bool,
) -> axum::http::HeaderValue {
    let tools = if webmcp_enabled {
        "tools=(self)"
    } else {
        "tools=()"
    };
    let value = format!(
        "camera=(), {}, geolocation=(), {tools}",
        channels::voice::microphone_permissions_policy_directive(voice_enabled),
    );
    axum::http::HeaderValue::from_str(&value)
        .expect("permissions policy value is assembled from static directives")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_consent_csp_allows_https_hops_beyond_the_redirect_origin() {
        let csp = oauth_consent_page_csp("https://www.cursor.com/agents/mcp/oauth/callback");
        assert!(csp.ends_with("form-action 'self' https:"), "got: {csp}");
    }

    #[test]
    fn oauth_consent_csp_allows_the_loopback_callback_origin() {
        let csp = oauth_consent_page_csp("http://127.0.0.1:53682/callback");
        assert!(
            csp.ends_with("form-action 'self' https: http://127.0.0.1:53682"),
            "got: {csp}"
        );
    }

    #[test]
    fn oauth_consent_csp_ignores_an_unparseable_redirect() {
        let csp = oauth_consent_page_csp("not a url");
        assert!(csp.ends_with("form-action 'self' https:"), "got: {csp}");
    }

    #[test]
    fn permissions_policy_denies_microphone_by_default() {
        assert_eq!(
            permissions_policy_header_value(false, false)
                .to_str()
                .unwrap(),
            "camera=(), microphone=(), geolocation=(), tools=()"
        );
    }

    #[test]
    fn permissions_policy_allows_microphone_when_voice_is_enabled() {
        assert_eq!(
            permissions_policy_header_value(true, false)
                .to_str()
                .unwrap(),
            "camera=(), microphone=(self), geolocation=(), tools=()"
        );
    }

    #[test]
    fn permissions_policy_allows_same_origin_webmcp_when_enabled() {
        assert_eq!(
            permissions_policy_header_value(false, true)
                .to_str()
                .unwrap(),
            "camera=(), microphone=(), geolocation=(), tools=(self)"
        );
    }
}
