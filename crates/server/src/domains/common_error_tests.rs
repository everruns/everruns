use super::*;

#[test]
fn command_metadata_declares_feature_gated_surfaces() {
    for (name, category, expected) in [
        ("list_evals", "evals", Some("evals")),
        ("list_skills", "skills", Some("skills")),
        ("list_memories", "memories", Some("memory")),
        (
            "list_knowledge_indexes",
            "knowledge_indexes",
            Some("knowledge"),
        ),
        ("list_plugins", "plugins", Some("plugins")),
        ("create_observer", "observers", Some("observers")),
        ("list_notifications", "notifications", Some("notifications")),
        (
            "list_payment_accounts",
            "payments",
            Some("machine_payments"),
        ),
        ("create_agent_version", "agents", Some("agent_versions")),
        ("list_agents", "agents", None),
    ] {
        let meta = CommandMeta {
            name,
            category,
            description: "",
            method: "GET",
            path: "",
        };
        assert_eq!(meta.required_feature(), expected, "command {name}");
    }
}

#[tokio::test]
async fn dispatch_accepts_empty_object_for_unit_commands() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = Ctx::minimal_for_test(Caller::internal(1), db, None);

    let result = dispatch("get_report_catalog", serde_json::json!({}), &ctx)
        .await
        .expect("empty MCP params should dispatch to a unit command");
    let value: serde_json::Value = serde_json::from_str(&result).expect("catalog JSON");

    assert!(value.get("datasets").is_some());
}

#[tokio::test]
async fn dispatch_does_not_coerce_nonempty_objects_for_unit_commands() {
    let db = Arc::new(StorageBackend::in_memory());
    let ctx = Ctx::minimal_for_test(Caller::internal(1), db, None);

    let error = dispatch(
        "get_report_catalog",
        serde_json::json!({ "unexpected": true }),
        &ctx,
    )
    .await
    .expect_err("non-empty params must remain invalid for a unit command");

    assert!(matches!(error.kind, CommandErrorKind::BadRequest(_)));
}

#[test]
fn classify_anyhow_maps_bad_request_error() {
    let err = classify_anyhow(crate::errors::BadRequestError::new("bad input").into());
    assert!(
        matches!(err, CommandError { kind: CommandErrorKind::BadRequest(msg), .. } if msg == "bad input")
    );
}

#[test]
fn classify_anyhow_maps_flattened_pool_exhaustion_to_service_unavailable() {
    let err = classify_anyhow(anyhow::anyhow!(
        "load request resource: pool timed out while waiting for an open connection"
    ));

    assert_eq!(err.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        err.code.as_deref(),
        Some(crate::errors::DATABASE_POOL_EXHAUSTED_CODE)
    );
}
#[test]
fn unique_conflict_http_response_redacts_database_details() {
    let raw = "error returned from database: duplicate key value violates unique constraint \
                   \"idx_mcp_servers_org_name_live\" at sqlx-postgres/src/connection.rs:666";
    let err = classify_anyhow(anyhow::anyhow!(raw));
    let (status, body) = <(StatusCode, Json<ErrorResponse>)>::from(err);

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body.0.detail.as_deref(), Some("Resource already exists"));
    assert_eq!(body.0.code.as_deref(), Some("already_exists"));
    assert!(!serde_json::to_string(&body.0).unwrap().contains(raw));
}

#[test]
fn domain_conflict_http_response_preserves_safe_detail() {
    let err = classify_anyhow(anyhow::anyhow!("Agent already exists"));
    let (status, body) = <(StatusCode, Json<ErrorResponse>)>::from(err);

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body.0.detail.as_deref(), Some("Agent already exists"));
    assert_eq!(body.0.code.as_deref(), Some("already_exists"));
}

// Cardinality contract for the `status` label on `everruns_commands_total`
// / `everruns_command_duration_seconds`. These strings are an external
// surface — Prometheus dashboards and alerts grep on them — so they must
// stay stable. Adding a CommandError variant requires a label here.
#[test]
fn command_error_status_label_covers_every_variant() {
    assert_eq!(
        command_error_status_label(&CommandError::bad_request("x")),
        "bad_request"
    );
    assert_eq!(
        command_error_status_label(&CommandError::unprocessable("x")),
        "unprocessable"
    );
    assert_eq!(
        command_error_status_label(&CommandError::forbidden("x")),
        "forbidden"
    );
    assert_eq!(
        command_error_status_label(&CommandError::not_found_msg("x")),
        "not_found"
    );
    assert_eq!(
        command_error_status_label(&CommandError::conflict("x")),
        "conflict"
    );
    assert_eq!(
        command_error_status_label(&CommandError::internal(anyhow::anyhow!("x"))),
        "internal"
    );
    assert_eq!(
        command_error_status_label(&CommandError::unavailable("x")),
        "unavailable"
    );
}

#[test]
fn classify_anyhow_maps_pool_timeout_to_service_unavailable() {
    let err = classify_anyhow(
        anyhow::Error::new(sqlx::Error::PoolTimedOut).context("load request resource"),
    );
    assert!(matches!(
        err,
        CommandError {
            kind: CommandErrorKind::Unavailable(_),
            ..
        }
    ));
    assert_eq!(err.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        err.code.as_deref(),
        Some(crate::errors::DATABASE_POOL_EXHAUSTED_CODE)
    );
    assert_eq!(
        err.retry_after_seconds,
        Some(crate::errors::DATABASE_POOL_RETRY_AFTER_SECONDS)
    );
}

#[test]
fn classify_anyhow_maps_not_found_error() {
    let err = classify_anyhow(crate::errors::ResourceNotFoundError::new("Thing").into());
    assert!(
        matches!(err, CommandError { kind: CommandErrorKind::NotFound(msg), .. } if msg == "Thing not found")
    );
}

// EVE-645: MCP server lookups raise `ResourceNotFoundError::new("MCP
// server")` (mcp_servers/service.rs) instead of a stringly-typed
// `anyhow!("MCP server not found")` that fell through to Internal/500.
// Pin that it now classifies as a 404.
#[test]
fn classify_anyhow_maps_mcp_server_not_found() {
    let err = classify_anyhow(crate::errors::ResourceNotFoundError::new("MCP server").into());
    let CommandError {
        kind: CommandErrorKind::NotFound(msg),
        ..
    } = &err
    else {
        panic!("MCP server not-found must classify as NotFound, got {err:?}");
    };
    assert_eq!(msg, "MCP server not found");
    assert_eq!(err.status(), StatusCode::NOT_FOUND);
}

// EVE-649: org MCP server create/update raise a stringly-typed
// `anyhow::bail!("stdio MCP servers are not supported for organization MCP
// servers")` on client-supplied transport config. Pin that it classifies
// as a 400 instead of falling through to Internal/500.
#[test]
fn classify_anyhow_maps_stdio_mcp_server_rejection_to_bad_request() {
    let err = classify_anyhow(anyhow::anyhow!(
        "stdio MCP servers are not supported for organization MCP servers"
    ));
    let CommandError {
        kind: CommandErrorKind::BadRequest(_),
        ..
    } = &err
    else {
        panic!("stdio MCP server rejection must classify as BadRequest, got {err:?}");
    };
    assert_eq!(err.status(), StatusCode::BAD_REQUEST);
}

// EVE-437: every substring added to the `is_bad_request` list must in
// fact map an `anyhow::bail!` to `CommandError::BadRequest` rather than
// silently 500ing. New entries here pin the contract.
#[test]
fn classify_anyhow_maps_eve437_validation_substrings() {
    let cases = [
        "Harness inheritance cycle detected",
        "Harness cannot inherit from itself",
        "Parent harness must be active",
        "Cannot archive or delete harness while child harnesses still inherit from it: a, b",
        "Could not find a unique name for 'foo'",
        "eval target must specify harness_id or harness_name",
        "App targets not yet supported in eval execution",
        "harness_id and harness_name are mutually exclusive in eval target",
        "Only API key MCP servers can store an API key",
        "API key auth mode requires an API key",
        "API key auth mode requires a non-empty API key",
        "OAuth MCP servers require a user connection before tools can be refreshed",
        // EVE-649: org MCP server transport validation
        "stdio MCP servers are not supported for organization MCP servers",
    ];
    for raw in cases {
        let err = classify_anyhow(anyhow::anyhow!("{raw}"));
        assert!(
            matches!(
                err,
                CommandError {
                    kind: CommandErrorKind::BadRequest(_),
                    ..
                }
            ),
            "{raw} must classify as BadRequest, got {err:?}"
        );
    }
}

// EVE-437: a generic anyhow error with no recognized pattern must keep
// mapping to Internal so we do not accidentally widen the bad-request
// class for unrelated future failures.
#[test]
fn classify_anyhow_unknown_message_is_internal() {
    let err = classify_anyhow(anyhow::anyhow!("connection timed out"));
    assert!(
        matches!(
            err,
            CommandError {
                kind: CommandErrorKind::Internal(_),
                ..
            }
        ),
        "unknown messages must remain Internal: {err:?}"
    );
}

// EVE-437: PolicyError stays mapped to Forbidden — this is the path used
// by the new sessions/service.rs high-risk capability check that used to
// be a bail! and 500'd.
#[test]
fn classify_anyhow_maps_policy_error() {
    let pe = everruns_core::PolicyError::denied("test_policy", "admin role");
    let err = classify_anyhow(anyhow::Error::new(pe));
    let CommandError {
        kind: CommandErrorKind::Forbidden(msg),
        ..
    } = &err
    else {
        panic!("PolicyError must classify as Forbidden, got {err:?}");
    };
    assert!(
        msg.contains("admin role"),
        "Forbidden message must include detail, got {msg:?}"
    );
}

#[test]
fn http_adapter_propagates_extensions() {
    use crate::api::common::AllowedAction;
    let err = CommandError::conflict("agent already exists")
        .with_code("agent_already_exists")
        .with_action(
            AllowedAction::new("get-existing")
                .with_operation_id("get_agent")
                .with_hint("Fetch the existing agent and reuse it."),
        );
    let (status, body) = <(StatusCode, Json<ErrorResponse>)>::from(err);
    assert_eq!(status, StatusCode::CONFLICT);
    let body = body.0;
    assert_eq!(body.status, 409);
    assert_eq!(body.title, "Conflict");
    assert_eq!(body.detail.as_deref(), Some("agent already exists"));
    assert_eq!(body.code.as_deref(), Some("agent_already_exists"));
    assert_eq!(body.allowed_actions.len(), 1);
    assert_eq!(body.allowed_actions[0].rel, "get-existing");
    assert_eq!(
        body.allowed_actions[0].operation_id.as_deref(),
        Some("get_agent")
    );
}

#[test]
fn http_adapter_propagates_retry_after() {
    let err = CommandError::unprocessable("backend warming up").with_retry_after(30);
    let (status, body) = <(StatusCode, Json<ErrorResponse>)>::from(err);
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body.0.retry_after_seconds, Some(30));
}

#[test]
fn http_adapter_redacts_internal_detail() {
    let err = CommandError::internal(anyhow::anyhow!("db handle exhausted: secret=YExample0"));
    let (_status, body) = <(StatusCode, Json<ErrorResponse>)>::from(err);
    // detail is the safe generic string, never the anyhow message.
    assert_eq!(body.0.detail.as_deref(), Some("Internal server error"));
    assert_eq!(body.0.code.as_deref(), Some("internal_error"));
}

#[test]
fn mcp_format_dispatch_error_unchanged_by_extensions() {
    // Bashkit consumers parse `<kind>: <message>` — adding extensions to
    // CommandError MUST NOT alter that wire string.
    let plain = CommandError::bad_request("missing field 'name'");
    let with_ext = CommandError::bad_request("missing field 'name'")
        .with_code("agent_name_required")
        .with_retry_after(0);
    assert_eq!(plain.to_string(), "missing field 'name'");
    assert_eq!(with_ext.to_string(), "missing field 'name'");
}
