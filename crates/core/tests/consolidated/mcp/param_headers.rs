//! `x-mcp-header` tool parameters travel as `Mcp-Param-*` headers (MCP
//! 2026-07-28 Streamable HTTP). The scripted server behaves like GitHub's MCP
//! server: it marks `owner`/`repo` and rejects a `tools/call` whose headers do
//! not mirror them with `400` / `-32020 HeaderMismatch`.
use async_trait::async_trait;
use everruns_core::mcp::{McpClient, McpConnection, NoAuthProvider};
use everruns_core::{
    EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const URL: &str = "http://8.8.8.8/mcp";

#[derive(Default)]
struct GitHubLike {
    requests: Mutex<Vec<(String, BTreeMap<String, String>)>>,
    /// Reject every call as a mismatch, whatever the headers say.
    reject_all: std::sync::atomic::AtomicBool,
}

impl GitHubLike {
    fn methods(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(m, _)| m.clone())
            .collect()
    }

    fn call_headers(&self) -> Vec<BTreeMap<String, String>> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "tools/call")
            .map(|(_, h)| h.clone())
            .collect()
    }
}

fn header<'a>(headers: &'a BTreeMap<String, String>, name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn tools() -> Value {
    json!({"tools": [
        {"name": "pull_request_read", "inputSchema": {"type": "object", "properties": {
            "owner": {"type": "string", "x-mcp-header": "owner"},
            "repo": {"type": "string", "x-mcp-header": "repo"},
            "pullNumber": {"type": "number"}
        }}},
        {"name": "search_issues", "inputSchema": {"type": "object", "properties": {
            "query": {"type": "string"}
        }}},
        {"name": "broken", "inputSchema": {"type": "object", "properties": {
            "items": {"type": "array", "items": {"type": "string", "x-mcp-header": "item"}}
        }}}
    ]})
}

#[async_trait]
impl EgressService for GitHubLike {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let method = body["method"].as_str().unwrap().to_string();
        self.requests
            .lock()
            .unwrap()
            .push((method.clone(), request.headers.clone()));
        let reply = |status: u16, payload: Value| {
            EgressResult::Ok(EgressResponse {
                status,
                headers: BTreeMap::new(),
                body: serde_json::to_vec(&payload).unwrap(),
            })
        };
        match method.as_str() {
            "tools/list" => reply(
                200,
                json!({"jsonrpc":"2.0","id":body["id"],"result":tools()}),
            ),
            "tools/call" => {
                let args = &body["params"]["arguments"];
                if self.reject_all.load(std::sync::atomic::Ordering::SeqCst) {
                    return reply(
                        400,
                        json!({"jsonrpc":"2.0","id":body["id"],"error":{"code":-32020,"message":"header mismatch"}}),
                    );
                }
                for (param, name) in [("owner", "Mcp-Param-owner"), ("repo", "Mcp-Param-repo")] {
                    let Some(expected) = args.get(param).and_then(Value::as_str) else {
                        continue;
                    };
                    if header(&request.headers, name) != Some(expected) {
                        return reply(
                            400,
                            json!({"jsonrpc":"2.0","id":body["id"],"error":{
                                "code":-32020,
                                "message":format!("header mismatch: missing {name} header for parameter \"{param}\"")
                            }}),
                        );
                    }
                }
                reply(
                    200,
                    json!({"jsonrpc":"2.0","id":body["id"],"result":{
                        "content":[{"type":"text","text":"ok"}]
                    }}),
                )
            }
            other => panic!("unexpected {other}"),
        }
    }

    async fn send_stream(&self, request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        let response = self.send(request).await?;
        Ok(EgressStreamResponse {
            status: response.status,
            headers: response.headers,
            body: Box::pin(futures::stream::iter([Ok(response.body)])),
        })
    }
}

fn setup() -> (Arc<GitHubLike>, McpClient, McpConnection) {
    let server = Arc::new(GitHubLike::default());
    let client = McpClient::new(server.clone(), Arc::new(NoAuthProvider));
    (server, client, McpConnection::http("github", URL))
}

#[tokio::test]
async fn discovery_drops_invalid_tools_and_calls_mirror_parameters() {
    let (server, client, connection) = setup();
    let names: Vec<_> = client
        .discover(&connection)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, ["pull_request_read", "search_issues"]);

    let args = json!({"owner": "everruns", "repo": "tuika", "pullNumber": 150});
    let result = client
        .call(&connection, "pull_request_read", args, None)
        .await
        .unwrap();
    assert!(!result.is_error);
    let calls = server.call_headers();
    assert_eq!(calls.len(), 1, "no retry needed after discovery");
    assert_eq!(header(&calls[0], "Mcp-Param-owner"), Some("everruns"));
    assert_eq!(header(&calls[0], "Mcp-Param-repo"), Some("tuika"));
    assert_eq!(header(&calls[0], "Mcp-Name"), Some("pull_request_read"));
}

#[tokio::test]
async fn a_rejected_call_rereads_tools_and_retries_once() {
    // A worker that never listed tools in this process: the first call has no
    // annotations to go on, the server says HeaderMismatch, the client
    // re-reads the schema and the retry carries the headers.
    let (server, client, connection) = setup();
    client
        .call(
            &connection,
            "pull_request_read",
            json!({"owner": "everruns", "repo": "everruns", "pullNumber": 1}),
            None,
        )
        .await
        .unwrap();
    assert_eq!(server.methods(), ["tools/call", "tools/list", "tools/call"]);
    let calls = server.call_headers();
    assert_eq!(header(&calls[0], "Mcp-Param-owner"), None);
    assert_eq!(header(&calls[1], "Mcp-Param-owner"), Some("everruns"));
}

#[tokio::test]
async fn tools_without_annotations_send_no_param_headers() {
    let (server, client, connection) = setup();
    client.discover(&connection).await.unwrap();
    client
        .call(
            &connection,
            "search_issues",
            json!({"query": "is:pr"}),
            None,
        )
        .await
        .unwrap();
    let calls = server.call_headers();
    assert!(
        calls[0]
            .keys()
            .all(|k| !k.to_ascii_lowercase().starts_with("mcp-param-"))
    );
}

#[tokio::test]
async fn an_argument_that_cannot_be_a_header_is_refused_before_sending() {
    // An argument that cannot become a header fails locally with a message
    // naming it, instead of a round trip the server is bound to reject.
    let (server, client, connection) = setup();
    client.discover(&connection).await.unwrap();
    let error = client
        .call(
            &connection,
            "pull_request_read",
            // `repo` is a list: no header can carry it, so the client refuses
            // before sending anything.
            json!({"owner": "everruns", "repo": ["everruns"]}),
            None,
        )
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("Mcp-Param-repo"));
    assert!(server.call_headers().is_empty());
}

#[tokio::test]
async fn a_mismatch_the_schema_cannot_explain_is_returned_not_looped() {
    // The re-read schema is the one already used, so retrying cannot help:
    // the error comes back after one tools/list, not a retry loop.
    let (server, client, connection) = setup();
    client.discover(&connection).await.unwrap();
    server
        .reject_all
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let error = client
        .call(
            &connection,
            "pull_request_read",
            json!({"owner": "everruns", "repo": "everruns"}),
            None,
        )
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("-32020"));
    assert_eq!(server.methods(), ["tools/list", "tools/call", "tools/list"]);
}
