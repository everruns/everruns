use super::*;

#[derive(Serialize)]
struct TestResource {
    id: String,
}

#[derive(Debug, Clone, Serialize)]
struct TestResourceWithActions {
    id: String,
}

impl ResourceUrlable for TestResourceWithActions {
    fn api_path() -> &'static str {
        "v1/test-resources"
    }

    fn ui_path() -> &'static str {
        "test-resources"
    }

    fn resource_id(&self) -> String {
        self.id.clone()
    }

    fn allowed_actions(&self, api_base: &str) -> Vec<AllowedAction> {
        vec![AllowedAction {
            rel: "update".to_string(),
            href: Some(format!("{api_base}/{}/{}", Self::api_path(), self.id)),
            method: Some("PATCH".to_string()),
            operation_id: Some("update_test_resource".to_string()),
            schema_ref: Some("#/components/schemas/UpdateTestResourceRequest".to_string()),
            hint: None,
        }]
    }
}

impl ResourceUrlable for TestResource {
    fn api_path() -> &'static str {
        "v1/test-resources"
    }

    fn ui_path() -> &'static str {
        "test-resources"
    }

    fn resource_id(&self) -> String {
        self.id.clone()
    }
}

// Trivial derive-only serde round-trips removed; covered by the derive + handler tests.

#[test]
fn test_error_response_into_response() {
    let (status, json) = ErrorResponse::new("Test").into_response(StatusCode::BAD_REQUEST);
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json.0.detail.as_deref(), Some("Test"));
    assert_eq!(json.0.title, "Bad Request");
    assert_eq!(json.0.status, 400);
}

#[test]
fn test_api_result_ext_log_internal_error() {
    let result: Result<i32, &str> = Err("db connection failed");
    let mapped = result.log_internal_error("get agent");
    assert_eq!(mapped.unwrap_err(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn internal_error_response_redacts_source_details() {
    let result: Result<i32, &str> = Err("postgres query failed: secret-database-marker");
    let (status, json) = result.log_internal_error_json("get model").unwrap_err();
    let response = (status, json).into_response();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["detail"], "Internal server error");
    assert_eq!(value["status"], 500);
    assert!(
        !String::from_utf8(bytes.to_vec())
            .unwrap()
            .contains("secret-database-marker")
    );
}

#[test]
fn classify_unique_conflict_redacts_database_details() {
    let raw = "error returned from database: duplicate key value violates unique constraint \
               \"idx_memories_org_name_active\" at sqlx-postgres/src/connection.rs:666";
    let error = anyhow::anyhow!(raw).context("create memory");
    let (status, body) = classify_anyhow_error(&error).expect("classified conflict");

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        body.0.detail.as_deref(),
        Some(crate::errors::ALREADY_EXISTS_DETAIL)
    );
    assert_eq!(
        body.0.code.as_deref(),
        Some(crate::errors::ALREADY_EXISTS_CODE)
    );
    assert!(!serde_json::to_string(&body.0).unwrap().contains(raw));
}

#[test]
fn classify_domain_conflict_preserves_safe_detail() {
    let error = anyhow::anyhow!("Memory already exists");
    let (status, body) = classify_anyhow_error(&error).expect("classified conflict");

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body.0.detail.as_deref(), Some("Memory already exists"));
    assert_eq!(
        body.0.code.as_deref(),
        Some(crate::errors::ALREADY_EXISTS_CODE)
    );
}

#[test]
fn test_api_option_ext_ok_or_not_found() {
    let some: Option<i32> = Some(42);
    assert_eq!(some.ok_or_not_found().unwrap(), 42);

    let none: Option<i32> = None;
    assert_eq!(none.ok_or_not_found().unwrap_err(), StatusCode::NOT_FOUND);
}

#[test]
fn test_api_option_ext_ok_or_not_found_json() {
    let some: Option<i32> = Some(42);
    assert_eq!(some.ok_or_not_found_json("Agent").unwrap(), 42);

    let none: Option<i32> = None;
    let (status, json) = none.ok_or_not_found_json("Agent").unwrap_err();
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json.0.detail.as_deref(), Some("Agent not found"));
}

#[test]
fn test_paginated_response_serialization() {
    let response = PaginatedResponse::new(vec!["a", "b"], 50, 10, 5);
    let json = serde_json::to_string(&response).unwrap();

    // Verify JSON structure
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["data"], serde_json::json!(["a", "b"]));
    assert_eq!(parsed["total"], 50);
    assert_eq!(parsed["offset"], 10);
    assert_eq!(parsed["limit"], 5);
    // next_url / prev_url are absent until the middleware fills them in.
    assert!(parsed.get("next_url").is_none());
    assert!(parsed.get("prev_url").is_none());
}

#[test]
fn inject_pagination_links_adds_next_and_prev_on_middle_page() {
    let api_base = "https://api.example/api";
    let mut value = serde_json::json!({"data": [1,2,3], "total": 100, "offset": 20, "limit": 10});
    let changed = inject_pagination_links(
        &mut value,
        api_base,
        "/v1/platform/agents?offset=20&limit=10",
    );
    assert!(changed);
    assert_eq!(
        value["next_url"],
        "https://api.example/api/v1/platform/agents?offset=30&limit=10"
    );
    assert_eq!(
        value["prev_url"],
        "https://api.example/api/v1/platform/agents?offset=10&limit=10"
    );
}

#[test]
fn inject_pagination_links_omits_next_on_last_page() {
    let mut value = serde_json::json!({"data": [], "total": 25, "offset": 20, "limit": 10});
    let changed = inject_pagination_links(
        &mut value,
        "https://api.example/api",
        "/v1/platform/agents?offset=20&limit=10",
    );
    assert!(changed);
    assert!(value.get("next_url").is_none());
    assert!(value.get("prev_url").is_some());
}

#[test]
fn inject_pagination_links_omits_prev_on_first_page() {
    let mut value = serde_json::json!({"data": [], "total": 25, "offset": 0, "limit": 10});
    let changed = inject_pagination_links(
        &mut value,
        "https://api.example/api",
        "/v1/platform/agents?offset=0&limit=10",
    );
    assert!(changed);
    assert!(value.get("next_url").is_some());
    assert!(value.get("prev_url").is_none());
}

#[test]
fn inject_pagination_links_preserves_other_query_params() {
    let mut value = serde_json::json!({"data": [], "total": 100, "offset": 10, "limit": 10});
    let changed = inject_pagination_links(
        &mut value,
        "https://api.example/api",
        "/v1/platform/agents?search=foo&offset=10&limit=10&include_archived=true",
    );
    assert!(changed);
    assert_eq!(
        value["next_url"],
        "https://api.example/api/v1/platform/agents?search=foo&offset=20&limit=10&include_archived=true"
    );
}

#[test]
fn inject_pagination_links_appends_offset_when_missing_from_query() {
    let mut value = serde_json::json!({"data": [], "total": 100, "offset": 0, "limit": 10});
    let changed = inject_pagination_links(
        &mut value,
        "https://api.example/api",
        "/v1/platform/agents?search=foo",
    );
    assert!(changed);
    assert_eq!(
        value["next_url"],
        "https://api.example/api/v1/platform/agents?search=foo&offset=10"
    );
}

#[test]
fn inject_pagination_links_skips_objects_without_pagination_fields() {
    let mut value = serde_json::json!({"data": [1,2,3]});
    let changed = inject_pagination_links(&mut value, "https://x", "/y");
    assert!(!changed);
    assert!(value.get("next_url").is_none());
}

#[test]
fn inject_pagination_links_preserves_handler_supplied_next_url() {
    let mut value = serde_json::json!({
        "data": [],
        "total": 100,
        "offset": 20,
        "limit": 10,
        "next_url": "https://custom/cursor-based-link"
    });
    let changed = inject_pagination_links(
        &mut value,
        "https://api.example/api",
        "/v1/platform/agents?offset=20&limit=10",
    );
    assert!(changed);
    // Handler-supplied next_url is preserved.
    assert_eq!(value["next_url"], "https://custom/cursor-based-link");
    // prev_url is still filled in by the middleware.
    assert_eq!(
        value["prev_url"],
        "https://api.example/api/v1/platform/agents?offset=10&limit=10"
    );
}

#[test]
fn inject_pagination_links_preserves_handler_supplied_prev_url() {
    let mut value = serde_json::json!({
        "data": [],
        "total": 100,
        "offset": 20,
        "limit": 10,
        "prev_url": "https://custom/cursor-based-link"
    });
    let changed = inject_pagination_links(
        &mut value,
        "https://api.example/api",
        "/v1/platform/agents?offset=20&limit=10",
    );
    assert!(changed);
    // Handler-supplied prev_url is preserved.
    assert_eq!(value["prev_url"], "https://custom/cursor-based-link");
    // next_url is still filled in by the middleware.
    assert_eq!(
        value["next_url"],
        "https://api.example/api/v1/platform/agents?offset=30&limit=10"
    );
}

#[test]
fn test_url_builder_wrap_includes_ui_link_alias() {
    let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
    let wrapped = builder.wrap(TestResource {
        id: "resource_1".to_string(),
    });

    assert_eq!(
        wrapped.self_url,
        "https://api.example/api/v1/test-resources/resource_1"
    );
    assert_eq!(
        wrapped.view_url,
        "https://app.example/test-resources/resource_1"
    );
    assert_eq!(wrapped.ui_link, wrapped.view_url);
}

#[test]
fn test_link_decoration_requires_bounded_content_length() {
    let missing_length = Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::empty())
        .unwrap();
    assert!(!response_within_link_decoration_limit(&missing_length));

    let large_response = Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::CONTENT_LENGTH,
            (LINK_DECORATION_MAX_RESPONSE_BYTES + 1).to_string(),
        )
        .body(Body::empty())
        .unwrap();
    assert!(!response_within_link_decoration_limit(&large_response));

    let bounded_response = Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, "128")
        .body(Body::empty())
        .unwrap();
    assert!(response_within_link_decoration_limit(&bounded_response));
}

#[test]
fn test_decorate_value_links_adds_command_session_link() {
    let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
    let mut value = serde_json::json!({
        "session_id": "session_00000000000000000000000000000001",
        "message_id": "message_00000000000000000000000000000002"
    });

    assert!(builder.decorate_value_links(&mut value));
    assert!(value.get("self_url").is_none());
    assert_eq!(
        value["ui_link"],
        "https://app.example/sessions/session_00000000000000000000000000000001/chat"
    );
}

#[test]
fn test_decorate_value_links_adds_nested_capability_link() {
    let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
    let mut value = serde_json::json!({
        "data": [{
            "id": "platform_management",
            "name": "Platform Management",
            "type": "builtin"
        }]
    });

    assert!(builder.decorate_value_links(&mut value));
    assert_eq!(
        value["data"][0]["view_url"],
        "https://app.example/capabilities/platform_management"
    );
}

#[test]
fn test_decorate_value_links_aligns_budget_ui_link() {
    let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
    let mut value = serde_json::json!({
        "id": "bdgt_00000000000000000000000000000001",
        "subject_type": "session"
    });

    assert!(builder.decorate_value_links(&mut value));
    assert_eq!(value["view_url"], "https://app.example/budgets");
    assert_eq!(value["ui_link"], value["view_url"]);
}

#[test]
fn test_decorate_value_links_preserves_existing_links() {
    let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
    let mut value = serde_json::json!({
        "id": "agent_00000000000000000000000000000001",
        "self_url": "https://custom.example/api",
        "view_url": "https://custom.example/view"
    });

    assert!(builder.decorate_value_links(&mut value));
    assert_eq!(value["self_url"], "https://custom.example/api");
    assert_eq!(value["view_url"], "https://custom.example/view");
    assert_eq!(value["ui_link"], "https://custom.example/view");
}

#[test]
fn test_decorate_value_links_aliases_existing_view_url() {
    let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
    let mut value = serde_json::json!({
        "id": "compaction",
        "name": "Compaction",
        "view_url": "https://app.example/capabilities/compaction"
    });

    assert!(builder.decorate_value_links(&mut value));
    assert_eq!(
        value["ui_link"],
        "https://app.example/capabilities/compaction"
    );
}

#[test]
fn problem_details_constructors_set_status_title_code() {
    let cases = [
        (
            ErrorResponse::not_found("Agent"),
            StatusCode::NOT_FOUND,
            "Not Found",
            "not_found",
            "Agent not found",
        ),
        (
            ErrorResponse::conflict("agent already exists"),
            StatusCode::CONFLICT,
            "Conflict",
            "conflict",
            "agent already exists",
        ),
        (
            ErrorResponse::internal_error(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
            "internal_error",
            "Internal server error",
        ),
        (
            ErrorResponse::bad_gateway(),
            StatusCode::BAD_GATEWAY,
            "Bad Gateway",
            "bad_gateway",
            "Bad gateway",
        ),
    ];
    for (i, (response, expected_status, title, code, detail)) in cases.iter().enumerate() {
        let (status, Json(body)) = response;
        assert_eq!(status, expected_status, "case {i} status");
        assert_eq!(body.title, *title, "case {i} title");
        assert_eq!(
            body.status,
            expected_status.as_u16(),
            "case {i} status code"
        );
        assert_eq!(body.code.as_deref(), Some(*code), "case {i} code");
        assert_eq!(body.detail.as_deref(), Some(*detail), "case {i} detail");
        assert!(body.allowed_actions.is_empty(), "case {i} actions");
        assert!(body.retry_after_seconds.is_none(), "case {i} retry");
    }
}

#[test]
fn problem_details_builders_add_extensions() {
    let response = ErrorResponse::new("Shorten 'name'")
        .with_title("Validation failed")
        .with_code("agent_name_too_long")
        .with_type("https://errors.everruns.com/validation/agent-name-too-long")
        .with_instance("/v1/platform/agents")
        .with_action(
            AllowedAction::new("retry")
                .with_operation_id("create_agent")
                .with_hint("Shorten 'name' to <= 200 chars."),
        )
        .into_response(StatusCode::UNPROCESSABLE_ENTITY);

    let value = serde_json::to_value(&response.1.0).unwrap();
    assert_eq!(value["title"], "Validation failed");
    assert_eq!(value["status"], 422);
    assert_eq!(value["detail"], "Shorten 'name'");
    assert_eq!(value["code"], "agent_name_too_long");
    assert_eq!(value["instance"], "/v1/platform/agents");
    assert_eq!(
        value["type"],
        "https://errors.everruns.com/validation/agent-name-too-long"
    );
    let actions = value["allowed_actions"].as_array().unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["rel"], "retry");
    assert_eq!(actions[0]["operation_id"], "create_agent");
}

#[test]
fn problem_details_omits_unset_optional_fields() {
    let (_, body) = ErrorResponse::not_found("Agent");
    let value = serde_json::to_value(&body.0).unwrap();
    assert!(value.get("type").is_none());
    assert!(value.get("instance").is_none());
    assert!(value.get("retry_after_seconds").is_none());
    assert!(value.get("allowed_actions").is_none());
}

#[test]
fn problem_details_rate_limit_carries_retry_after() {
    let body = ErrorResponse::new("Too many requests.")
        .with_code("rate_limited")
        .with_retry_after(60)
        .with_action(
            AllowedAction::new("retry-later")
                .with_hint("Wait retry_after_seconds before retrying."),
        )
        .into_response(StatusCode::TOO_MANY_REQUESTS);
    let value = serde_json::to_value(&body.1.0).unwrap();
    assert_eq!(value["status"], 429);
    assert_eq!(value["code"], "rate_limited");
    assert_eq!(value["retry_after_seconds"], 60);
    assert_eq!(value["allowed_actions"][0]["rel"], "retry-later");
}

#[test]
fn session_actions_include_cancel_when_turn_is_active() {
    use crate::domains::sessions::record::SessionStatus;
    let actions = session_allowed_actions(
        "session_01",
        &SessionStatus::Active,
        false,
        false,
        "https://api.example",
    );
    let rels: Vec<&str> = actions.iter().map(|a| a.rel.as_str()).collect();
    assert!(
        rels.contains(&"cancel"),
        "active turn should expose cancel: {rels:?}"
    );
    let cancel = actions.iter().find(|a| a.rel == "cancel").unwrap();
    assert_eq!(cancel.method.as_deref(), Some("POST"));
    assert_eq!(cancel.operation_id.as_deref(), Some("cancel_turn"));
    assert_eq!(
        cancel.href.as_deref(),
        Some("https://api.example/v1/platform/sessions/session_01/cancel")
    );
}

#[test]
fn session_actions_omit_cancel_in_idle_or_paused_states() {
    use crate::domains::sessions::record::SessionStatus;
    for status in [
        SessionStatus::Started,
        SessionStatus::Idle,
        SessionStatus::WaitingForToolResults,
        SessionStatus::Paused,
    ] {
        let actions =
            session_allowed_actions("session_01", &status, false, false, "https://api.example");
        assert!(
            actions.iter().all(|a| a.rel != "cancel"),
            "{status:?}: cancel must not appear when no turn is mid-flight"
        );
    }
}

#[test]
fn session_actions_flip_pin_rel_on_is_pinned() {
    use crate::domains::sessions::record::SessionStatus;
    let unpinned = session_allowed_actions(
        "session_01",
        &SessionStatus::Idle,
        false,
        false,
        "https://api.example",
    );
    let pinned = session_allowed_actions(
        "session_01",
        &SessionStatus::Idle,
        true,
        false,
        "https://api.example",
    );
    let unpinned_rels: Vec<&str> = unpinned.iter().map(|a| a.rel.as_str()).collect();
    let pinned_rels: Vec<&str> = pinned.iter().map(|a| a.rel.as_str()).collect();
    assert!(
        unpinned_rels.contains(&"pin") && !unpinned_rels.contains(&"unpin"),
        "unpinned session offers pin, not unpin: {unpinned_rels:?}"
    );
    assert!(
        pinned_rels.contains(&"unpin") && !pinned_rels.contains(&"pin"),
        "pinned session offers unpin, not pin: {pinned_rels:?}"
    );
    let pin = unpinned.iter().find(|a| a.rel == "pin").unwrap();
    assert_eq!(pin.method.as_deref(), Some("PUT"));
    let unpin = pinned.iter().find(|a| a.rel == "unpin").unwrap();
    assert_eq!(unpin.method.as_deref(), Some("DELETE"));
}

#[test]
fn session_actions_flip_archive_rel_on_is_archived() {
    use crate::domains::sessions::record::SessionStatus;
    let active = session_allowed_actions(
        "session_01",
        &SessionStatus::Idle,
        false,
        false,
        "https://api.example",
    );
    let archived = session_allowed_actions(
        "session_01",
        &SessionStatus::Idle,
        false,
        true,
        "https://api.example",
    );
    let active_rels: Vec<&str> = active.iter().map(|a| a.rel.as_str()).collect();
    let archived_rels: Vec<&str> = archived.iter().map(|a| a.rel.as_str()).collect();
    assert!(
        active_rels.contains(&"archive") && !active_rels.contains(&"unarchive"),
        "active session offers archive, not unarchive: {active_rels:?}"
    );
    assert!(
        archived_rels.contains(&"unarchive") && !archived_rels.contains(&"archive"),
        "archived session offers unarchive, not archive: {archived_rels:?}"
    );
    let archive = active.iter().find(|a| a.rel == "archive").unwrap();
    assert_eq!(archive.method.as_deref(), Some("PUT"));
    let unarchive = archived.iter().find(|a| a.rel == "unarchive").unwrap();
    assert_eq!(unarchive.method.as_deref(), Some("DELETE"));
}

#[test]
fn session_actions_always_include_self_events_stream_update_delete() {
    use crate::domains::sessions::record::SessionStatus;
    let actions = session_allowed_actions(
        "session_xyz",
        &SessionStatus::Idle,
        false,
        false,
        "https://api.example",
    );
    let rels: Vec<&str> = actions.iter().map(|a| a.rel.as_str()).collect();
    for expected in ["self", "events", "stream", "update", "delete"] {
        assert!(
            rels.contains(&expected),
            "missing rel `{expected}` in {rels:?}"
        );
    }
    let update = actions.iter().find(|a| a.rel == "update").unwrap();
    assert_eq!(
        update.schema_ref.as_deref(),
        Some("#/components/schemas/UpdateSessionRequest"),
        "update action must reference its request body schema"
    );
    // `events` is JSON polling (operationId list_events); `stream` is SSE
    // (operationId stream_sse, /sse path).
    let events = actions.iter().find(|a| a.rel == "events").unwrap();
    assert_eq!(events.operation_id.as_deref(), Some("list_events"));
    assert!(events.href.as_deref().unwrap().ends_with("/events"));
    let stream = actions.iter().find(|a| a.rel == "stream").unwrap();
    assert_eq!(stream.operation_id.as_deref(), Some("stream_sse"));
    assert!(stream.href.as_deref().unwrap().ends_with("/sse"));
}

#[test]
fn with_urls_preserves_allowed_actions_for_resource_with_counts() {
    let builder = UrlBuilder::new("https://api.example", "https://app.example");
    let wrapped = builder.wrap(ResourceWithCounts {
        session_count: 1,
        inner: TestResourceWithActions {
            id: "resource_1".to_string(),
        },
    });
    assert_eq!(wrapped.allowed_actions.len(), 1);
    assert_eq!(wrapped.allowed_actions[0].rel, "update");
}

#[test]
fn with_urls_skips_empty_allowed_actions() {
    let builder = UrlBuilder::new("https://api.example/api", "https://app.example");
    let wrapped = builder.wrap(TestResource {
        id: "resource_1".to_string(),
    });
    let value = serde_json::to_value(&wrapped).unwrap();
    assert!(
        value.get("allowed_actions").is_none(),
        "resources that didn't opt in must not emit allowed_actions: {value}"
    );
}

// ----------------------------------------------------------------
// Hypermedia rollout — EVE-494
// ----------------------------------------------------------------

#[test]
fn agent_actions_offer_copy_only_when_active() {
    use crate::domains::agents::record::AgentStatus;
    let active = agent_allowed_actions("agent_01", &AgentStatus::Active, "https://api.example");
    let archived = agent_allowed_actions("agent_01", &AgentStatus::Archived, "https://api.example");
    assert!(
        active.iter().any(|a| a.rel == "copy"),
        "active agent should expose copy"
    );
    assert!(
        !archived.iter().any(|a| a.rel == "copy"),
        "archived agent should not expose copy"
    );
    let rels: Vec<&str> = active.iter().map(|a| a.rel.as_str()).collect();
    for expected in ["self", "update", "history", "delete"] {
        assert!(
            rels.contains(&expected),
            "missing rel `{expected}` for active agent: {rels:?}"
        );
    }
    let update = active.iter().find(|a| a.rel == "update").unwrap();
    assert_eq!(
        update.schema_ref.as_deref(),
        Some("#/components/schemas/UpdateAgentRequest")
    );
}

#[test]
fn harness_actions_offer_copy_only_when_active() {
    use crate::domains::harnesses::record::HarnessStatus;
    let active =
        harness_allowed_actions("harness_01", &HarnessStatus::Active, "https://api.example");
    let archived = harness_allowed_actions(
        "harness_01",
        &HarnessStatus::Archived,
        "https://api.example",
    );
    assert!(active.iter().any(|a| a.rel == "copy"));
    assert!(!archived.iter().any(|a| a.rel == "copy"));
}

#[test]
fn skill_actions_omit_delete_for_terminal_states() {
    use crate::domains::skills::record::SkillStatus;
    for status in [SkillStatus::Active, SkillStatus::Disabled] {
        let actions = skill_allowed_actions("skill_01", &status, "https://api.example");
        assert!(
            actions.iter().any(|a| a.rel == "delete"),
            "{status:?}: expected delete"
        );
    }
    for status in [SkillStatus::Archived, SkillStatus::Deleted] {
        let actions = skill_allowed_actions("skill_01", &status, "https://api.example");
        assert!(
            !actions.iter().any(|a| a.rel == "delete"),
            "{status:?}: delete should not appear on already-terminal states"
        );
    }
}
