#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]
//! A small in-process Kubernetes API for the `kubernetes` sandbox tests.
//!
//! It stores pods, persistent volume claims, secrets and network policies as
//! JSON, answers the calls the provider makes (create, get, list by label,
//! delete, server-side apply, the pod proxy), and runs the real sandbox guest
//! for every pod, on its own loopback address (`127.0.0.N`) and the shared
//! guest port. The pod's workspace is a real directory named by the pod's
//! `EVERRUNS_GUEST_WORKSPACE`; it survives the pod, like a volume.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use base64::Engine;
use everruns_integrations::sandbox_guest::server::{GuestConfig, router};
use serde_json::{Value, json};

#[derive(Default)]
pub struct Cluster {
    /// `(kind, name)` → object.
    pub objects: Mutex<BTreeMap<(String, String), Value>>,
    guests: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    next_ip: Mutex<u8>,
    pub guest_port: u16,
    /// Images that fail to pull.
    pub broken_images: Mutex<Vec<String>>,
    /// Every request, as `METHOD path`.
    pub calls: Mutex<Vec<String>>,
}

impl Cluster {
    pub fn object(&self, kind: &str, name: &str) -> Option<Value> {
        self.objects
            .lock()
            .unwrap()
            .get(&(kind.to_string(), name.to_string()))
            .cloned()
    }

    pub fn names(&self, kind: &str) -> Vec<String> {
        self.objects
            .lock()
            .unwrap()
            .keys()
            .filter(|(k, _)| k == kind)
            .map(|(_, name)| name.clone())
            .collect()
    }

    /// Evict a pod the way the kubelet does: it stays, phase `Failed`.
    pub fn evict(&self, name: &str) {
        let mut objects = self.objects.lock().unwrap();
        if let Some(pod) = objects.get_mut(&("pods".to_string(), name.to_string())) {
            pod["status"] =
                json!({"phase": "Failed", "reason": "Evicted", "message": "low memory"});
        }
        if let Some(guest) = self.guests.lock().unwrap().remove(name) {
            guest.abort();
        }
    }
}

pub struct MockCluster {
    pub url: String,
    pub state: Arc<Cluster>,
}

fn status(code: u16, reason: &str, message: impl Into<String>) -> Response {
    (
        StatusCode::from_u16(code).unwrap(),
        axum::Json(json!({
            "kind": "Status", "apiVersion": "v1", "metadata": {},
            "status": "Failure", "reason": reason, "code": code, "message": message.into(),
        })),
    )
        .into_response()
}

fn matches(object: &Value, selector: Option<&str>) -> bool {
    let Some(selector) = selector else {
        return true;
    };
    selector
        .split(',')
        .filter(|pair| !pair.is_empty())
        .all(|pair| {
            let (key, value) = pair.split_once('=').unwrap();
            object["metadata"]["labels"][key] == json!(value)
        })
}

fn list_kind(kind: &str) -> &'static str {
    match kind {
        "pods" => "PodList",
        "persistentvolumeclaims" => "PersistentVolumeClaimList",
        "secrets" => "SecretList",
        _ => "NetworkPolicyList",
    }
}

/// `stringData` becomes base64 `data`, as the API server does.
fn normalize_secret(object: &mut Value) {
    if let Some(string_data) = object.get("stringData").cloned() {
        let data: serde_json::Map<String, Value> = string_data
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    json!(
                        base64::engine::general_purpose::STANDARD
                            .encode(value.as_str().unwrap_or_default())
                    ),
                )
            })
            .collect();
        object["data"] = Value::Object(data);
        object.as_object_mut().unwrap().remove("stringData");
    }
}

fn env_value(pod: &Value, name: &str) -> Option<String> {
    pod["spec"]["containers"][0]["env"]
        .as_array()?
        .iter()
        .find(|entry| entry["name"] == name)
        .and_then(|entry| entry["value"].as_str())
        .map(str::to_string)
}

async fn start_pod(state: &Arc<Cluster>, pod: &mut Value) {
    let name = pod["metadata"]["name"].as_str().unwrap().to_string();
    let image = pod["spec"]["containers"][0]["image"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if state.broken_images.lock().unwrap().contains(&image) {
        pod["status"] = json!({
            "phase": "Pending",
            "containerStatuses": [{
                "name": "sandbox", "ready": false, "restartCount": 0, "image": image, "imageID": "",
                "state": {"waiting": {"reason": "ErrImagePull", "message": "pull access denied"}},
            }],
        });
        return;
    }
    let secret_name = format!("{name}-guest");
    let secret = state
        .object("secrets", &secret_name)
        .expect("guest secret first");
    let token = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(secret["data"]["token"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    let workspace = env_value(pod, "EVERRUNS_GUEST_WORKSPACE").unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    state.objects.lock().unwrap().insert(
        ("_workspaces".to_string(), format!("{name}-workspace")),
        json!(workspace),
    );
    let ip = {
        let mut next = state.next_ip.lock().unwrap();
        *next += 1;
        format!("127.0.0.{}", *next + 1)
    };
    // The exec environment lives in the secret; the guest reads it from a
    // file, so write the secret's copy next to the workspace.
    let env_file = std::path::PathBuf::from(format!("{workspace}.{name}.env"));
    let exec_env = secret["data"]["exec.env"]
        .as_str()
        .map(|data| {
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .unwrap()
        })
        .unwrap_or_default();
    std::fs::write(&env_file, exec_env).unwrap();
    let app = router(GuestConfig {
        token,
        workspace: workspace.into(),
        exec_env_file: Some(env_file),
        desktop: false,
    });
    let listener = tokio::net::TcpListener::bind(format!("{ip}:{}", state.guest_port))
        .await
        .unwrap();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    state.guests.lock().unwrap().insert(name, handle);
    pod["status"] = json!({
        "phase": "Running",
        "podIP": ip,
        "conditions": [
            {"type": "PodScheduled", "status": "True"},
            {"type": "Ready", "status": "True"},
        ],
    });
}

#[derive(serde::Deserialize)]
struct ListQuery {
    #[serde(rename = "labelSelector")]
    label_selector: Option<String>,
}

async fn collection(
    State(state): State<Arc<Cluster>>,
    Path((_ns, kind)): Path<(String, String)>,
    Query(query): Query<ListQuery>,
    request: Request<Body>,
) -> Response {
    let method = request.method().clone();
    state.calls.lock().unwrap().push(format!("{method} {kind}"));
    if method == "GET" {
        let items: Vec<Value> = state
            .objects
            .lock()
            .unwrap()
            .iter()
            .filter(|((k, _), object)| {
                k == &kind && matches(object, query.label_selector.as_deref())
            })
            .map(|(_, object)| object.clone())
            .collect();
        return axum::Json(json!({
            "apiVersion": "v1", "kind": list_kind(&kind),
            "metadata": {"resourceVersion": "1"}, "items": items,
        }))
        .into_response();
    }
    let body = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .unwrap();
    let mut object: Value = serde_json::from_slice(&body).unwrap();
    let name = object["metadata"]["name"].as_str().unwrap().to_string();
    if state.object(&kind, &name).is_some() {
        return status(409, "AlreadyExists", format!("{kind} {name} exists"));
    }
    object["metadata"]["uid"] = json!(uuid::Uuid::new_v4().to_string());
    object["metadata"]["resourceVersion"] = json!("1");
    object["metadata"]["creationTimestamp"] = json!("2026-11-15T00:00:00Z");
    if kind == "secrets" {
        normalize_secret(&mut object);
    }
    if kind == "pods" {
        start_pod(&state, &mut object).await;
    }
    state
        .objects
        .lock()
        .unwrap()
        .insert((kind, name), object.clone());
    (StatusCode::CREATED, axum::Json(object)).into_response()
}

async fn item(
    State(state): State<Arc<Cluster>>,
    Path((_ns, kind, name)): Path<(String, String, String)>,
    request: Request<Body>,
) -> Response {
    let method = request.method().clone();
    state
        .calls
        .lock()
        .unwrap()
        .push(format!("{method} {kind}/{name}"));
    let key = (kind.clone(), name.clone());
    match method.as_str() {
        "GET" => match state.object(&kind, &name) {
            Some(object) => axum::Json(object).into_response(),
            None => status(404, "NotFound", format!("{kind} \"{name}\" not found")),
        },
        "DELETE" => {
            let removed = state.objects.lock().unwrap().remove(&key);
            if kind == "pods"
                && let Some(guest) = state.guests.lock().unwrap().remove(&name)
            {
                guest.abort();
            }
            if kind == "persistentvolumeclaims"
                && let Some(workspace) = state.object("_workspaces", &name)
            {
                let _ = std::fs::remove_dir_all(workspace.as_str().unwrap());
            }
            match removed {
                Some(object) => axum::Json(object).into_response(),
                None => status(404, "NotFound", format!("{kind} \"{name}\" not found")),
            }
        }
        "PATCH" => {
            let body = axum::body::to_bytes(request.into_body(), usize::MAX)
                .await
                .unwrap();
            let mut object: Value = serde_json::from_slice(&body).unwrap();
            if kind == "secrets" {
                normalize_secret(&mut object);
            }
            let mut objects = state.objects.lock().unwrap();
            if let Some(existing) = objects.get(&key) {
                object["metadata"]["uid"] = existing["metadata"]["uid"].clone();
            }
            object["metadata"]["resourceVersion"] = json!("2");
            objects.insert(key, object.clone());
            axum::Json(object).into_response()
        }
        _ => status(405, "MethodNotAllowed", "unsupported"),
    }
}

/// `pods/<name>:<port>/proxy/<rest>`, forwarded to the pod's guest.
async fn pod_proxy(
    State(state): State<Arc<Cluster>>,
    Path((_ns, target, rest)): Path<(String, String, String)>,
    request: Request<Body>,
) -> Response {
    let (name, port) = target.split_once(':').unwrap();
    state.calls.lock().unwrap().push(format!("PROXY {name}"));
    let Some(pod) = state.object("pods", name) else {
        return status(404, "NotFound", "pod not found");
    };
    let Some(ip) = pod["status"]["podIP"].as_str() else {
        return status(503, "ServiceUnavailable", "pod has no IP");
    };
    let query = request
        .uri()
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    let method = reqwest::Method::from_bytes(request.method().as_str().as_bytes()).unwrap();
    let headers = request.headers().clone();
    let body: Bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .unwrap();
    let mut forwarded = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .request(method, format!("http://{ip}:{port}/{rest}{query}"))
        .body(body.to_vec());
    for (key, value) in &headers {
        // The API server's own credentials stop here.
        if key != "authorization" && key != "host" {
            forwarded = forwarded.header(key.as_str(), value.as_bytes());
        }
    }
    match forwarded.send().await {
        Ok(response) => {
            let code = response.status().as_u16();
            let bytes = response.bytes().await.unwrap_or_default();
            (StatusCode::from_u16(code).unwrap(), bytes.to_vec()).into_response()
        }
        Err(err) => status(503, "ServiceUnavailable", err.to_string()),
    }
}

pub async fn start() -> MockCluster {
    // One port for every pod's guest, each on its own loopback address.
    let port = {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        probe.local_addr().unwrap().port()
    };
    let state = Arc::new(Cluster {
        guest_port: port,
        ..Cluster::default()
    });
    let app = Router::new()
        .route(
            "/api/v1/namespaces/{ns}/pods/{target}/proxy/{*rest}",
            any(pod_proxy),
        )
        .route(
            "/api/v1/namespaces/{ns}/{kind}",
            get(collection).post(collection),
        )
        .route(
            "/api/v1/namespaces/{ns}/{kind}/{name}",
            get(item).delete(item).patch(item),
        )
        .route(
            "/apis/networking.k8s.io/v1/namespaces/{ns}/{kind}",
            get(collection).post(collection),
        )
        .route(
            "/apis/networking.k8s.io/v1/namespaces/{ns}/{kind}/{name}",
            get(item).delete(item).patch(item),
        )
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    MockCluster { url, state }
}

impl MockCluster {
    pub fn client(&self) -> kube::Client {
        let config = kube::Config::new(self.url.parse().unwrap());
        kube::Client::try_from(config).unwrap()
    }
}
