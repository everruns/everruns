use super::*;

#[tokio::test]
async fn unsupported_shapes_and_cost_options_refuse_before_admission_or_credentials() {
    for extra in [
        "organization",
        "session_id",
        "generation_id",
        "model",
        "metadata",
        "extra_headers",
        "tools",
        "plugins",
        "previous_response_id",
        "provider",
        "prompt_cache",
    ] {
        let mut value =
            json!({"messages":[{"role":"user","content":"hello"}],"max_output_tokens":32});
        value[extra] = json!("forged");
        assert!(
            serde_json::from_value::<TextRequest>(value).is_err(),
            "{extra}"
        );
    }
    for value in [
        json!({"messages":[{"role":"tool","content":"hello"}],"max_output_tokens":32}),
        json!({"messages":[{"role":"user","content":[{"type":"image_url","image_url":"http://foreign"}]}],"max_output_tokens":32}),
    ] {
        assert!(serde_json::from_value::<TextRequest>(value).is_err());
    }
    let host = Host::new();
    let transport = Transport::new(Reply::Json(response()));
    let mut p = policy();
    p.model = "openrouter/auto".into();
    assert_eq!(
        dispatch_text(&host, &transport.client(), &p, &request()).await,
        Err(DispatchError::Unsupported)
    );
    let mut p = policy();
    p.max_request_bytes = 100;
    assert_eq!(
        dispatch_text(&host, &transport.client(), &p, &request()).await,
        Err(DispatchError::Unsupported)
    );
    let mut r = request();
    r.max_output_tokens = 129;
    assert_eq!(
        dispatch_text(&host, &transport.client(), &policy(), &r).await,
        Err(DispatchError::Unsupported)
    );
    assert!(host.state.lock().unwrap().hash.is_none());
    assert_eq!(host.auth_calls.load(Ordering::SeqCst), 0);
    assert!(transport.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn revocation_and_foreign_endpoint_never_resolve_credentials_or_send() {
    let host = Host::new();
    host.state.lock().unwrap().revoke_after_reserve = true;
    let transport = Transport::new(Reply::Json(response()));
    assert_eq!(
        dispatch_text(&host, &transport.client(), &policy(), &request()).await,
        Err(DispatchError::Refused)
    );
    assert_eq!(host.auth_calls.load(Ordering::SeqCst), 0);
    let mut foreign = Host::new();
    foreign.endpoint = "https://foreign.invalid".into();
    assert_eq!(
        dispatch_text(&foreign, &transport.client(), &policy(), &request()).await,
        Err(DispatchError::Uncertain)
    );
    assert_eq!(foreign.auth_calls.load(Ordering::SeqCst), 0);
    assert!(foreign.state.lock().unwrap().held);
    assert!(transport.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn uncertain_send_redirect_and_oversized_response_retain_funding_without_retries() {
    for reply in [
        Reply::Failure,
        Reply::Status(302),
        Reply::Status(503),
        Reply::Oversized,
    ] {
        let host = Host::new();
        let transport = Transport::new(reply);
        assert_eq!(
            dispatch_text(&host, &transport.client(), &policy(), &request()).await,
            Err(DispatchError::Uncertain)
        );
        assert!(host.state.lock().unwrap().held);
        assert!(host.state.lock().unwrap().records.is_empty());
        assert_eq!(
            dispatch_text(&host, &transport.client(), &policy(), &request()).await,
            Err(DispatchError::Refused)
        );
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test(start_paused = true)]
async fn timeout_drops_the_reader_and_does_not_release_or_retry_the_operation() {
    let host = Host::new();
    let transport = Transport::new(Reply::Pending);
    assert_eq!(
        dispatch_text(&host, &transport.client(), &policy(), &request()).await,
        Err(DispatchError::Uncertain)
    );
    assert!(transport.dropped.load(Ordering::SeqCst));
    assert!(host.state.lock().unwrap().held);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn caller_cancellation_retains_the_started_operation_and_drops_the_reader() {
    let host = Host::new();
    let transport = Transport::new(Reply::Pending);
    let client = transport.client();
    let p = policy();
    let r = request();
    let mut dispatch = Box::pin(dispatch_text(&host, &client, &p, &r));
    tokio::select! {
        result=&mut dispatch => panic!("unexpected result: {result:?}"),
        _=transport.sent.notified()=>{}
    }
    drop(dispatch);
    assert!(transport.dropped.load(Ordering::SeqCst));
    assert!(host.state.lock().unwrap().held);
    assert_eq!(
        dispatch_text(&host, &client, &p, &r).await,
        Err(DispatchError::Refused)
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn canonical_incomplete_unexpected_or_over_bound_usage_requires_durable_review() {
    for case in 0..11 {
        let mut response = response();
        let expected = match case {
            0 => {
                response["usage"].as_object_mut().unwrap().remove("cost");
                ReceiptReview::IncompleteUsage
            }
            1 => {
                response["usage"]["completion_tokens"] = json!(10000);
                ReceiptReview::ExceededBound
            }
            2 => {
                response["usage"]["cost"] = json!(1);
                ReceiptReview::ExceededBound
            }
            3 => {
                response["openrouter_metadata"]["endpoints"]["available"][0]["provider"] =
                    json!("foreign");
                ReceiptReview::UnexpectedShape
            }
            4 => {
                response["choices"][0]["message"]["tool_calls"] = json!([{"id":"hidden-tool"}]);
                ReceiptReview::UnexpectedShape
            }
            5 => {
                response["usage"]["cost"] = json!(-0.01);
                ReceiptReview::IncompleteUsage
            }
            6 => {
                response["usage"]["completion_tokens"] = json!(-1);
                ReceiptReview::IncompleteUsage
            }
            7 => {
                response["usage"] = json!([]);
                ReceiptReview::IncompleteUsage
            }
            8 => {
                response["model"] = json!([]);
                ReceiptReview::UnexpectedShape
            }
            9 => {
                response["openrouter_metadata"]["attempt"] = json!(2);
                ReceiptReview::UnexpectedShape
            }
            _ => {
                response["usage"]["server_tool_use_details"] =
                    json!({"tool_calls_requested":1,"tool_calls_executed":1});
                ReceiptReview::UnexpectedShape
            }
        };
        let host = Host::new();
        let transport = Transport::new(Reply::Json(response));
        assert_eq!(
            dispatch_text(&host, &transport.client(), &policy(), &request()).await,
            Err(DispatchError::Review)
        );
        let state = host.state.lock().unwrap();
        assert!(state.held);
        assert_eq!(state.records.len(), 1);
        assert_eq!(state.records[0].5, Some(expected));
    }
}

#[tokio::test]
async fn journal_failure_cannot_return_output_or_fall_back_to_another_dispatch() {
    let host = Host::new();
    host.state.lock().unwrap().fail_record = true;
    let transport = Transport::new(Reply::Json(response()));
    assert_eq!(
        dispatch_text(&host, &transport.client(), &policy(), &request()).await,
        Err(DispatchError::Uncertain)
    );
    assert!(host.state.lock().unwrap().held);
    assert!(host.state.lock().unwrap().records.is_empty());
    assert_eq!(
        dispatch_text(&host, &transport.client(), &policy(), &request()).await,
        Err(DispatchError::Refused)
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}
