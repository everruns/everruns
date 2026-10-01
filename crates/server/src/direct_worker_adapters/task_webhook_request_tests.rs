use super::build_task_webhook_request;

#[test]
fn task_webhook_request_pins_dns_and_signs() {
    // EVE-625: delivery must pin DNS to defeat create-time->delivery DNS
    // rebinding SSRF.
    let req = build_task_webhook_request(
        "https://hooks.example.com/notify",
        br#"{"event":"task.terminal"}"#,
        Some("s3cret"),
    );
    assert!(
        req.dns_pinning_required,
        "task webhook delivery must require DNS pinning"
    );
    assert!(
        req.headers.iter().any(
            |(k, v)| k.eq_ignore_ascii_case("X-Everruns-Signature") && v.starts_with("sha256=")
        ),
        "signed webhook must carry an HMAC signature header"
    );

    // Unsigned webhook still pins DNS and omits the signature header.
    let unsigned = build_task_webhook_request("https://hooks.example.com/notify", b"{}", None);
    assert!(unsigned.dns_pinning_required);
    assert!(
        !unsigned
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("X-Everruns-Signature"))
    );
}

use super::spec_push_config_targets;
use crate::storage::session_task_store::TaskTransition;

#[test]
fn spec_push_configs_filter_by_event() {
    // EVE-682: spawn-time configs embedded in the task spec deliver only for
    // events their event_filter includes; an absent filter defaults to
    // terminal-only, matching org webhook behavior.
    let spec = serde_json::json!({
        "push_configs": [
            { "url": "https://a.example.com", "secret": "s", "event_filter": ["message"] },
            { "url": "https://b.example.com", "event_filter": ["terminal", "awaiting_input"] },
            { "url": "https://c.example.com" },
        ]
    });

    let terminal = spec_push_config_targets(&spec, TaskTransition::Terminal);
    let terminal_urls: Vec<&str> = terminal.iter().map(|t| t.url.as_str()).collect();
    assert_eq!(
        terminal_urls,
        vec!["https://b.example.com", "https://c.example.com"],
        "terminal must include the explicit terminal filter and the default"
    );

    let message = spec_push_config_targets(&spec, TaskTransition::Message);
    let message_urls: Vec<&str> = message.iter().map(|t| t.url.as_str()).collect();
    assert_eq!(message_urls, vec!["https://a.example.com"]);
    assert_eq!(
        message[0].secret.as_deref(),
        Some("s"),
        "secret must be carried through for signing"
    );

    let awaiting = spec_push_config_targets(&spec, TaskTransition::AwaitingInput);
    let awaiting_urls: Vec<&str> = awaiting.iter().map(|t| t.url.as_str()).collect();
    assert_eq!(awaiting_urls, vec!["https://b.example.com"]);

    // No push_configs key → no targets.
    assert!(spec_push_config_targets(&serde_json::json!({}), TaskTransition::Terminal).is_empty());
}
