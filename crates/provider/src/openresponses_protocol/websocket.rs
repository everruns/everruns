// OpenAI Responses API WebSocket mode: when a call uses it.
//
// The Responses API accepts the same create body over a persistent WebSocket
// to `/v1/responses` (a `response.create` client event) and streams back the
// same events it sends over SSE. Keeping one socket across the turns of a tool
// loop lets OpenAI continue from connection-local state instead of reloading
// the previous response; OpenAI recommends it for `service_tier: "ultrafast"`.
// Wire contract and what was inferred: knowledge/foundations/openai-responses-websocket.md.
//
// Decisions:
// - Opt-in. SSE stays the default transport. A driver must declare support
//   (`with_websocket_support`, set by the OpenAI driver for `api.openai.com`
//   only), and then a call opts in with the `openai/websocket` driver option,
//   or the driver turns it on for every call (`with_websocket_default`) and a
//   call opts out with `false`.
// - SSE is also the fallback: a socket that fails to connect, or that drops or
//   answers with an `error` event before the first response event, costs one
//   failed attempt and the call proceeds over HTTP unchanged. That keeps every
//   HTTP-side behaviour (status classification, 429 retries, the stateless
//   recovery for a rejected continuation) without re-implementing it here.
// - Background mode is HTTP-only (the WebSocket API rejects `background`), so
//   a background call never uses the socket.
// - The transport itself needs the `responses-websocket` crate feature; built
//   without it, the option is accepted and ignored (SSE).

use serde_json::Value;

use crate::driver_registry::LlmCallConfig;

/// Driver option opting one call into (`true`) or out of (`false`) the
/// Responses WebSocket transport. Ignored by drivers without WebSocket support.
pub const OPENAI_WEBSOCKET_OPTION: &str = "openai/websocket";

/// Whether, and by default, a driver uses the WebSocket transport.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct WebSocketPolicy {
    /// The endpoint implements Responses WebSocket mode.
    pub(crate) supported: bool,
    /// Calls use it unless they opt out.
    pub(crate) default_on: bool,
}

impl WebSocketPolicy {
    /// Whether this call goes over the WebSocket transport.
    // Without the transport feature only the tests ask.
    #[cfg_attr(not(feature = "responses-websocket"), allow(dead_code))]
    pub(crate) fn wants(&self, config: &LlmCallConfig, background: bool) -> bool {
        if !cfg!(feature = "responses-websocket") || !self.supported || background {
            return false;
        }
        config
            .driver_options
            .get(OPENAI_WEBSOCKET_OPTION)
            .and_then(Value::as_bool)
            .unwrap_or(self.default_on)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn config(option: Option<bool>) -> LlmCallConfig {
        let mut config = LlmCallConfig::default();
        if let Some(value) = option {
            config
                .driver_options
                .insert(OPENAI_WEBSOCKET_OPTION.into(), json!(value));
        }
        config
    }

    #[test]
    fn websocket_is_opt_in_and_never_used_without_support_or_in_background() {
        let off = WebSocketPolicy::default();
        let supported = WebSocketPolicy {
            supported: true,
            default_on: false,
        };
        let default_on = WebSocketPolicy {
            supported: true,
            default_on: true,
        };
        let compiled = cfg!(feature = "responses-websocket");

        // Unsupported endpoints ignore the option entirely.
        assert!(!off.wants(&config(Some(true)), false));
        // Supported endpoints stay on SSE until a call opts in.
        assert!(!supported.wants(&config(None), false));
        assert_eq!(supported.wants(&config(Some(true)), false), compiled);
        // A driver-wide default can be overridden per call.
        assert_eq!(default_on.wants(&config(None), false), compiled);
        assert!(!default_on.wants(&config(Some(false)), false));
        // Background mode is HTTP-only.
        assert!(!default_on.wants(&config(Some(true)), true));
    }
}
