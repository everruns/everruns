//! Tests: manifest_build.

use super::*;
use everruns_core::channel::SessionBinding;
use everruns_worker::AgentRunner;

use super::tests_support::*;

#[test]
fn test_thread_context_triggers_only_for_per_thread_with_thread_ts() {
    // PerThread + thread_ts present → should trigger context injection
    let config = test_config(SessionBinding::Thread);
    let event = test_event("C123", Some("1234.5678"), Some("1234.0000"));
    assert!(config.session_strategy == SessionBinding::Thread && event.thread_ts.is_some());

    // PerThread + no thread_ts → should NOT trigger (new thread, no prior context)
    let event_no_thread = test_event("C123", Some("1234.5678"), None);
    assert!(event_no_thread.thread_ts.is_none());

    // PerChannel → should NOT trigger
    let config_channel = test_config(SessionBinding::Conversation);
    assert!(config_channel.session_strategy != SessionBinding::Thread);

    // PerUser → should NOT trigger
    let config_user = test_config(SessionBinding::Requester);
    assert!(config_user.session_strategy != SessionBinding::Thread);
}

#[test]
fn test_manifest_includes_history_scopes_for_thread_context() {
    // conversations.replies requires channels:history / groups:history / im:history / mpim:history
    let yaml = build_manifest_yaml(
        "Bot",
        "Bot",
        None,
        "https://example.com/api/v1/apps/app_x/slack/events",
        "https://example.com/api/v1/apps/app_x/slack/interactivity",
        false,
        &[],
    );
    assert!(
        yaml.contains("channels:history"),
        "Manifest must include channels:history for conversations.replies"
    );
    assert!(
        yaml.contains("groups:history"),
        "Manifest must include groups:history for private channel thread context"
    );
    assert!(
        yaml.contains("im:history"),
        "Manifest must include im:history for DM thread context"
    );
    assert!(
        yaml.contains("mpim:history"),
        "Manifest must include mpim:history for group DM thread context"
    );
}

#[async_trait::async_trait]
impl AgentRunner for NoopRunner {
    async fn start_run(
        &self,
        _org_id: i64,
        _session_id: everruns_provider::typed_id::SessionId,
        _harness_id: everruns_provider::typed_id::HarnessId,
        _agent_id: Option<everruns_provider::typed_id::AgentId>,
        _input_message_id: everruns_provider::typed_id::MessageId,
        _request_id: Option<String>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn resume_after_tool_results(
        &self,
        _session_id: everruns_provider::typed_id::SessionId,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn cancel_run(
        &self,
        _run_id: everruns_provider::typed_id::SessionId,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn is_running(&self, _run_id: everruns_provider::typed_id::SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

// ==========================================
// WireMock integration tests — Slack API
// ==========================================

mod wiremock_tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ------------------------------------------
    // resolve_slack_user_name — success paths
    // ------------------------------------------

    #[tokio::test]
    async fn test_resolve_user_name_display_name() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users.info"))
            .and(header("Authorization", "Bearer xoxb-test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "user": {
                    "id": "U123",
                    "name": "alice",
                    "real_name": "Alice Smith",
                    "profile": {
                        "display_name": "Alice S."
                    }
                }
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache: SlackUserCache = new_slack_user_cache();
        let result =
            resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                .await;

        assert_eq!(result, Some("Alice S.".to_string()));

        // Verify it was cached
        let cached = cache.get("U123");
        assert_eq!(cached, Some(Some("Alice S.".to_string())));
    }

    #[tokio::test]
    async fn test_resolve_user_name_fallback_to_real_name() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users.info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "user": {
                    "id": "U123",
                    "name": "alice",
                    "real_name": "Alice Smith",
                    "profile": {
                        "display_name": ""
                    }
                }
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache: SlackUserCache = new_slack_user_cache();
        let result =
            resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                .await;

        // Empty display_name → falls back to real_name
        assert_eq!(result, Some("Alice Smith".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_user_name_fallback_to_name() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users.info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "user": {
                    "id": "U123",
                    "name": "alice",
                    "profile": {
                        "display_name": ""
                    }
                }
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache: SlackUserCache = new_slack_user_cache();
        let result =
            resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                .await;

        // No real_name → falls back to name
        assert_eq!(result, Some("alice".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_user_name_no_profile() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users.info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": true,
                "user": {
                    "id": "U123",
                    "name": "alice"
                }
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache: SlackUserCache = new_slack_user_cache();
        let result =
            resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                .await;

        // No profile → falls back to name
        assert_eq!(result, Some("alice".to_string()));
    }

    // ------------------------------------------
    // resolve_slack_user_name — cache behavior
    // ------------------------------------------

    #[tokio::test]
    async fn test_resolve_user_name_uses_cache() {
        let mock_server = MockServer::start().await;

        // No mock mounted — any actual HTTP call would fail
        let cache: SlackUserCache = new_slack_user_cache();
        cache.insert("U123".to_string(), Some("Cached Alice".to_string()));

        let result =
            resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                .await;

        assert_eq!(result, Some("Cached Alice".to_string()));
        // No HTTP call made (mock server received 0 requests)
    }

    #[test]
    fn test_slack_user_cache_is_bounded() {
        let cache: SlackUserCache = new_slack_user_cache();
        let inserts = (SLACK_USER_CACHE_MAX_ENTRIES as usize) + 5_000;
        for index in 0..inserts {
            cache.insert(format!("U{index}"), Some(format!("user-{index}")));
        }

        assert!(cache.entry_count() <= SLACK_USER_CACHE_MAX_ENTRIES);
    }

    #[tokio::test]
    async fn test_resolve_user_name_cached_none_skips_api() {
        let mock_server = MockServer::start().await;

        // Cache a None (permanent failure like missing_scope)
        let cache: SlackUserCache = new_slack_user_cache();
        cache.insert("U123".to_string(), None);

        let result =
            resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                .await;

        assert_eq!(result, None);
    }

    // ------------------------------------------
    // resolve_slack_user_name — error paths
    // ------------------------------------------

    #[tokio::test]
    async fn test_resolve_user_name_missing_scope_cached() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users.info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false,
                "error": "missing_scope"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache: SlackUserCache = new_slack_user_cache();
        let result =
            resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                .await;

        assert_eq!(result, None);

        // Permanent error should be cached as None
        let cached = cache.get("U123");
        assert_eq!(cached, Some(None));
    }

    #[tokio::test]
    async fn test_resolve_user_name_invalid_auth_cached() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users.info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false,
                "error": "invalid_auth"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache: SlackUserCache = new_slack_user_cache();
        let result =
            resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-bad-token", "U123")
                .await;

        assert_eq!(result, None);
        assert_eq!(cache.get("U123"), Some(None));
    }

    #[tokio::test]
    async fn test_resolve_user_name_token_revoked_cached() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users.info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false,
                "error": "token_revoked"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache: SlackUserCache = new_slack_user_cache();
        let result =
            resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-revoked", "U123").await;

        assert_eq!(result, None);
        assert_eq!(cache.get("U123"), Some(None));
    }

    #[tokio::test]
    async fn test_resolve_user_name_transient_error_not_cached() {
        let mock_server = MockServer::start().await;

        // Transient error (e.g. internal_error) should NOT be cached
        Mock::given(method("GET"))
            .and(path("/users.info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false,
                "error": "internal_error"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache: SlackUserCache = new_slack_user_cache();
        let result =
            resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                .await;

        assert_eq!(result, None);

        // Transient error should NOT be cached (allow retry)
        assert!(cache.get("U123").is_none());
    }

    #[tokio::test]
    async fn test_resolve_user_name_malformed_json() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users.info"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache: SlackUserCache = new_slack_user_cache();
        let result =
            resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                .await;

        assert_eq!(result, None);
        // Parse failure is transient — not cached
        assert!(cache.get("U123").is_none());
    }

    #[tokio::test]
    async fn test_resolve_user_name_account_inactive_cached() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users.info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false,
                "error": "account_inactive"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache: SlackUserCache = new_slack_user_cache();
        let result =
            resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                .await;

        assert_eq!(result, None);
        assert_eq!(cache.get("U123"), Some(None));
    }

    #[tokio::test]
    async fn test_resolve_user_name_not_authed_cached() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/users.info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false,
                "error": "not_authed"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let cache: SlackUserCache = new_slack_user_cache();
        let result =
            resolve_slack_user_name_base(&mock_server.uri(), &cache, "xoxb-test-token", "U123")
                .await;

        assert_eq!(result, None);
        assert_eq!(cache.get("U123"), Some(None));
    }

    // ------------------------------------------
    // post_to_slack via slack_delivery module
    // ------------------------------------------

    #[tokio::test]
    async fn test_post_to_slack_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/chat.postMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false,
                "error": "channel_not_found"
            })))
            .expect(1)
            .mount(&mock_server)
            .await;

        let result = crate::slack_delivery::post_to_slack_base(
            &mock_server.uri(),
            "xoxb-test-token",
            "C_INVALID",
            "",
            "Hello!",
        )
        .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("channel_not_found")
        );
    }

    // ------------------------------------------
    // fetch_thread_replies — pagination (EVE-969)
    // ------------------------------------------

    /// Build a conversations.replies page of `count` messages.
    fn replies_page(start: usize, count: usize, next_cursor: Option<&str>) -> serde_json::Value {
        let messages: Vec<serde_json::Value> = (start..start + count)
            .map(|i| {
                serde_json::json!({
                    "user": "U123",
                    "text": format!("message {}", i),
                    "ts": format!("{}.000000", 1000 + i),
                })
            })
            .collect();
        let mut body = serde_json::json!({ "ok": true, "messages": messages });
        if let Some(cursor) = next_cursor {
            body["response_metadata"] = serde_json::json!({ "next_cursor": cursor });
        }
        body
    }

    /// The bug: a thread longer than one page was silently cut to 100.
    /// 250 messages must arrive whole, which requires following two cursors.
    #[tokio::test]
    async fn test_fetch_thread_replies_follows_cursor_to_end_of_thread() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/conversations.replies"))
            .and(wiremock::matchers::query_param_is_missing("cursor"))
            .respond_with(ResponseTemplate::new(200).set_body_json(replies_page(
                0,
                100,
                Some("c1"),
            )))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/conversations.replies"))
            .and(wiremock::matchers::query_param("cursor", "c1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(replies_page(
                100,
                100,
                Some("c2"),
            )))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/conversations.replies"))
            .and(wiremock::matchers::query_param("cursor", "c2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(replies_page(200, 50, None)))
            .mount(&mock_server)
            .await;

        let backfill =
            fetch_thread_replies_base(&mock_server.uri(), "xoxb-test-token", "C123", "1000.000000")
                .await;

        assert_eq!(
            backfill.messages.len(),
            250,
            "whole thread must be returned"
        );
        assert!(backfill.exhausted);
        assert_eq!(backfill.omitted_older, 0);
        assert!(!backfill.is_truncated());
        // Chronological order preserved across page boundaries.
        assert_eq!(backfill.messages[0].text.as_deref(), Some("message 0"));
        assert_eq!(backfill.messages[249].text.as_deref(), Some("message 249"));
    }

    /// An empty cursor string is Slack's "no more pages", not a page to fetch.
    #[tokio::test]
    async fn test_fetch_thread_replies_treats_empty_cursor_as_end() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/conversations.replies"))
            .respond_with(ResponseTemplate::new(200).set_body_json(replies_page(0, 100, Some(""))))
            .expect(1)
            .mount(&mock_server)
            .await;

        let backfill =
            fetch_thread_replies_base(&mock_server.uri(), "xoxb-test-token", "C123", "1000.000000")
                .await;

        assert_eq!(backfill.messages.len(), 100);
        assert!(backfill.exhausted);
    }

    /// Past the cap the newest messages are kept and the drop is counted,
    /// so the notice can say how much is missing instead of guessing.
    #[tokio::test]
    async fn test_fetch_thread_replies_caps_and_keeps_newest() {
        let mock_server = MockServer::start().await;
        let total_pages = (THREAD_BACKFILL_MAX_MESSAGES / 100) + 2; // 7 pages = 700 messages

        for page in 0..total_pages {
            let cursor_in = if page == 0 {
                None
            } else {
                Some(format!("c{}", page))
            };
            let cursor_out = if page + 1 == total_pages {
                None
            } else {
                Some(format!("c{}", page + 1))
            };
            let body = replies_page(page * 100, 100, cursor_out.as_deref());
            let mut mock = Mock::given(method("GET")).and(path("/conversations.replies"));
            mock = match cursor_in {
                Some(ref c) => mock.and(wiremock::matchers::query_param("cursor", c.as_str())),
                None => mock.and(wiremock::matchers::query_param_is_missing("cursor")),
            };
            mock.respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&mock_server)
                .await;
        }

        let backfill =
            fetch_thread_replies_base(&mock_server.uri(), "xoxb-test-token", "C123", "1000.000000")
                .await;

        let fetched = total_pages * 100;
        assert_eq!(backfill.messages.len(), THREAD_BACKFILL_MAX_MESSAGES);
        assert_eq!(
            backfill.omitted_older,
            fetched - THREAD_BACKFILL_MAX_MESSAGES
        );
        assert!(backfill.exhausted);
        assert!(backfill.is_truncated());
        // The tail is what survives: the newest message is still present.
        assert_eq!(
            backfill.messages.last().unwrap().text.as_deref(),
            Some(format!("message {}", fetched - 1).as_str())
        );
    }

    /// A cursor that never terminates must not page forever.
    #[tokio::test]
    async fn test_fetch_thread_replies_stops_at_page_cap() {
        let mock_server = MockServer::start().await;

        // Every page hands back a fresh cursor, so only the cap ends this.
        Mock::given(method("GET"))
            .and(path("/conversations.replies"))
            .respond_with(ResponseTemplate::new(200).set_body_json(replies_page(
                0,
                100,
                Some("always"),
            )))
            .expect(THREAD_BACKFILL_MAX_PAGES as u64)
            .mount(&mock_server)
            .await;

        let backfill =
            fetch_thread_replies_base(&mock_server.uri(), "xoxb-test-token", "C123", "1000.000000")
                .await;

        assert!(!backfill.exhausted, "page cap must be reported, not hidden");
        assert!(backfill.is_truncated());
        assert_eq!(backfill.messages.len(), THREAD_BACKFILL_MAX_MESSAGES);
    }

    /// A mid-thread failure keeps the pages already read rather than
    /// throwing away history the agent could still use.
    #[tokio::test]
    async fn test_fetch_thread_replies_keeps_earlier_pages_on_later_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/conversations.replies"))
            .and(wiremock::matchers::query_param_is_missing("cursor"))
            .respond_with(ResponseTemplate::new(200).set_body_json(replies_page(
                0,
                100,
                Some("c1"),
            )))
            .mount(&mock_server)
            .await;

        Mock::given(method("GET"))
            .and(path("/conversations.replies"))
            .and(wiremock::matchers::query_param("cursor", "c1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false,
                "error": "ratelimited"
            })))
            .mount(&mock_server)
            .await;

        let backfill =
            fetch_thread_replies_base(&mock_server.uri(), "xoxb-test-token", "C123", "1000.000000")
                .await;

        assert_eq!(backfill.messages.len(), 100);
        assert!(
            !backfill.exhausted,
            "partial history must read as truncated"
        );
        assert!(backfill.is_truncated());
    }

    /// A first-page error still degrades to "no history", as before.
    #[tokio::test]
    async fn test_fetch_thread_replies_empty_on_first_page_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/conversations.replies"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ok": false,
                "error": "missing_scope"
            })))
            .mount(&mock_server)
            .await;

        let backfill =
            fetch_thread_replies_base(&mock_server.uri(), "xoxb-test-token", "C123", "1000.000000")
                .await;

        assert!(backfill.messages.is_empty());
        // Nothing was dropped, so nothing to warn the agent about.
        assert!(backfill.exhausted);
        assert!(!backfill.is_truncated());
    }

    /// The notice must name the omitted count when we know it, and admit
    /// uncertainty when the page cap means we do not.
    #[test]
    fn test_truncation_notice_distinguishes_known_and_unknown_loss() {
        let capped = ThreadBackfill {
            messages: vec![],
            omitted_older: 312,
            exhausted: true,
        };
        let notice = truncation_notice(&capped);
        assert!(
            notice.contains("312 earlier messages were omitted"),
            "{notice}"
        );

        let unbounded = ThreadBackfill {
            messages: vec![],
            omitted_older: 0,
            exhausted: false,
        };
        let notice = truncation_notice(&unbounded);
        assert!(
            notice.contains("more recent messages may be missing"),
            "{notice}"
        );
    }
}
