//! Browser security headers served with the UI.
//!
// Split out of `app_builder.rs` (EVE-1047): that file is pinned by the size
// ratchet, and this is the most self-contained thing in it — a pure function
// over the voice flag, with no dependency on how the app is composed.

use crate::channels;

// TM-WEB-004/005: baseline CSP stamped on every response that does not set its
// own. `frame-src 'self' data:` lets the file-preview UI embed PDFs via a
// `data:application/pdf` iframe (sandboxed viewers don't render in Chromium)
// and keeps `about:srcdoc` previews (SVG/HTML/MCP cards) working under 'self'.
// `form-action` must remain the LAST directive: the MCP OAuth consent page
// removes it while retaining every other directive.
pub(crate) const BASE_CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; frame-src 'self' data:; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

/// CSP for the MCP OAuth consent page, whose confirm POST redirects to the client.
///
/// Browsers apply `form-action` to *every* redirect after form submission. A
/// registered callback can itself redirect to another origin or local port;
/// constraining the chain makes the click silently stop after the server has
/// already granted access. The page has one same-origin form, no scripts, and
/// escapes all client-controlled text. Keep the global policy for other pages.
pub(crate) fn oauth_consent_page_csp() -> &'static str {
    BASE_CONTENT_SECURITY_POLICY
        .strip_suffix("; form-action 'self'")
        .expect("baseline CSP ends with form-action")
}

/// `tools=(self)` lets a browser agent use the WebMCP tools the authenticated
/// UI registers, and only those: other origins and frames get none. WebMCP has
/// no feature flag; every tool that changes something asks the person first.
pub(crate) fn permissions_policy_header_value(voice_enabled: bool) -> axum::http::HeaderValue {
    let value = format!(
        "camera=(), {}, geolocation=(), tools=(self)",
        channels::voice::microphone_permissions_policy_directive(voice_enabled),
    );
    axum::http::HeaderValue::from_str(&value)
        .expect("permissions policy value is assembled from static directives")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_consent_csp_does_not_restrict_callback_chain() {
        let csp = oauth_consent_page_csp();
        assert!(!csp.contains("form-action"), "got: {csp}");
        assert!(csp.contains("frame-ancestors 'none'"), "got: {csp}");
        assert!(BASE_CONTENT_SECURITY_POLICY.ends_with("form-action 'self'"));
    }

    #[test]
    fn permissions_policy_denies_microphone_and_allows_same_origin_webmcp() {
        assert_eq!(
            permissions_policy_header_value(false).to_str().unwrap(),
            "camera=(), microphone=(), geolocation=(), tools=(self)"
        );
    }

    #[test]
    fn permissions_policy_allows_microphone_when_voice_is_enabled() {
        assert_eq!(
            permissions_policy_header_value(true).to_str().unwrap(),
            "camera=(), microphone=(self), geolocation=(), tools=(self)"
        );
    }
}
