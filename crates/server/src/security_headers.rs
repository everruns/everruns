//! Browser security headers served with the UI.
//!
//! Split out of `app_builder.rs` (EVE-1047): that file is pinned by the size
//! ratchet, and this is the most self-contained thing in it — a pure function
//! over two feature flags, with no dependency on how the app is composed.

use crate::api;

pub(crate) const BASE_CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; frame-src 'self' data:; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

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
        api::voice::microphone_permissions_policy_directive(voice_enabled),
    );
    axum::http::HeaderValue::from_str(&value)
        .expect("permissions policy value is assembled from static directives")
}

#[cfg(test)]
mod tests {
    use super::*;

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
