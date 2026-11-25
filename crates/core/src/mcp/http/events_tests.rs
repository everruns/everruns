use super::*;
use crate::{EgressResponse, EgressResult};
use std::sync::Mutex;

struct EventsEgress {
    requests: Mutex<Vec<EgressRequest>>,
    legacy: bool,
    extension: bool,
    schema: Value,
}
impl EventsEgress {
    fn new(legacy: bool) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            legacy,
            extension: true,
            schema: serde_json::json!({}),
        }
    }
}

#[async_trait]
impl EgressService for EventsEgress {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let method = body["method"].as_str().unwrap();
        self.requests.lock().unwrap().push(request.clone());
        let caps = if self.extension {
            serde_json::json!({"extensions":{"io.modelcontextprotocol/events":{}}})
        } else {
            // The obsolete draft capability must not enable the extension.
            serde_json::json!({"events":{}})
        };
        let result = match method {
            "initialize" => {
                serde_json::json!({"protocolVersion":"2025-11-25", "capabilities":caps})
            }
            "notifications/initialized" => serde_json::json!({}),
            _ if self.legacy && request.headers["MCP-Protocol-Version"] == "2026-07-28" => {
                return Ok(EgressResponse {
                    status: 400,
                    headers: BTreeMap::new(),
                    body: b"Session required; initialize first".to_vec(),
                });
            }
            "server/discover" => serde_json::json!({"capabilities":caps}),
            "events/list" => {
                serde_json::json!({"events":[{"name":"issue.created", "delivery":["webhook"], "inputSchema":self.schema},{"name":"пошта.received", "delivery":["webhook"], "inputSchema":self.schema}]})
            }
            _ => serde_json::json!({}),
        };
        Ok(EgressResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&if self.legacy && method == "server/discover" {
                serde_json::json!({"jsonrpc":"2.0", "id":body["id"], "error":{"code":-32601,"message":"method not found"}})
            } else {
                serde_json::json!({"jsonrpc":"2.0", "id":body["id"], "result":result})
            }).unwrap(),
        })
    }
    async fn send_stream(
        &self,
        request: EgressRequest,
    ) -> EgressResult<crate::EgressStreamResponse> {
        let response = self.send(request).await?;
        Ok(crate::EgressStreamResponse {
            status: response.status,
            headers: response.headers,
            body: Box::pin(futures::stream::iter([Ok(response.body)])),
        })
    }
}
async fn request(egress: &EventsEgress, method: &str, params: Value) -> Result<Value> {
    http_request(
        egress,
        "https://8.8.8.8/mcp",
        &HashMap::new(),
        None,
        method,
        &params,
    )
    .await
}

#[tokio::test]
async fn events_requests_route_by_name_including_base64_and_legacy_fallback() {
    for legacy in [false, true] {
        for name in ["issue.created", "пошта.received"] {
            for method in ["events/subscribe", "events/unsubscribe"] {
                let egress = EventsEgress::new(legacy);
                request(&egress, method, serde_json::json!({"name":name}))
                    .await
                    .unwrap();
                let requests = egress.requests.lock().unwrap();
                let named: Vec<_> = requests
                    .iter()
                    .filter(|r| {
                        serde_json::from_slice::<Value>(&r.body).unwrap()["method"] == method
                    })
                    .collect();
                assert!(!named.is_empty());
                for request in named {
                    assert_eq!(
                        request.headers.get("Mcp-Name"),
                        Some(&param_headers::encode_value(name))
                    );
                }
                assert_eq!(
                    requests.iter().any(
                        |r| serde_json::from_slice::<Value>(&r.body).unwrap()["method"]
                            == "initialize"
                    ),
                    legacy
                );
            }
        }
    }
}

#[tokio::test]
async fn events_list_has_no_name_header() {
    let egress = EventsEgress::new(false);
    request(&egress, "events/list", serde_json::json!({}))
        .await
        .unwrap();
    assert!(
        egress
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| !r.headers.contains_key("Mcp-Name"))
    );
}

#[tokio::test]
async fn no_events_request_without_extension_in_either_era() {
    for legacy in [false, true] {
        let mut egress = EventsEgress::new(legacy);
        egress.extension = false;
        let error = request(
            &egress,
            "events/subscribe",
            serde_json::json!({"name":"issue.created"}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.downcast_ref::<McpRpcError>().unwrap().code, -32601);
        assert!(egress.requests.lock().unwrap().iter().all(|r| {
            !serde_json::from_slice::<Value>(&r.body).unwrap()["method"]
                .as_str()
                .unwrap()
                .starts_with("events/")
        }));
    }
}

#[tokio::test]
async fn events_arguments_mirror_safe_headers_and_exclude_invalid_annotations() {
    let mut egress = EventsEgress::new(false);
    egress.schema = serde_json::json!({"type":"object", "properties":{"region":{"type":"string", "x-mcp-header":"Region"}}});
    request(&egress, "events/subscribe", serde_json::json!({"name":"issue.created", "arguments":{"region":"eu\r\nAuthorization: injected"}})).await.unwrap();
    assert_eq!(
        egress.requests.lock().unwrap().last().unwrap().headers["Mcp-Param-Region"],
        param_headers::encode_value("eu\r\nAuthorization: injected")
    );
    egress.schema["properties"]["region"]["x-mcp-header"] = serde_json::json!("Invalid\r\nName");
    let list = request(&egress, "events/list", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(list["events"], serde_json::json!([]));
    assert!(
        request(
            &egress,
            "events/subscribe",
            serde_json::json!({"name":"issue.created"})
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn static_connection_headers_cannot_override_event_routing() {
    let egress = EventsEgress::new(false);
    let headers = HashMap::from([
        ("mcp-name".into(), "wrong".into()),
        ("Mcp-Param-Region".into(), "stale".into()),
    ]);
    http_request(
        &egress,
        "https://8.8.8.8/mcp",
        &headers,
        None,
        "events/list",
        &serde_json::json!({}),
    )
    .await
    .unwrap();
    assert!(egress.requests.lock().unwrap().iter().all(|request| {
        request.headers.keys().all(|name| {
            !name.eq_ignore_ascii_case("mcp-name") && !name.eq_ignore_ascii_case("Mcp-Param-Region")
        })
    }));
}
