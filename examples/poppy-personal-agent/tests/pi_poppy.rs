#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The example against an independent company: pi-poppy's `CompanyServer`
//! (github.com/richardanaya/pi-poppy), run by
//! `scripts/poppy-interop/company.ts`, with `gpt-6.1-sol` as the personal
//! agent. Runs when `PI_POPPY_DIR` and `OPENAI_API_KEY` are both set
//! (`doppler run -- scripts/poppy-interop.sh`).
//!
//! The company lives on loopback under a test CA, which the agent's default
//! egress rightly refuses, so requests go through a client that trusts that
//! CA and resolves the test names to loopback. Everything else is the
//! example as it runs.

use std::collections::BTreeMap;

use everruns::Engine;
use everruns::poppy::{
    EgressError, EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse,
    PoppyKey, PoppyPersonalAgent,
};
use everruns_poppy_personal_agent::{identity_routes, openai_model, personal_agent, poppy_calls};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};

const COMPANY_HOST: &str = "shop.poppy.localhost";
const AGENT_HOST: &str = "agent.poppy.localhost";

struct ToTestCompany {
    client: reqwest::Client,
}

#[async_trait::async_trait]
impl EgressService for ToTestCompany {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| EgressError::InvalidRequest(error.to_string()))?;
        let mut outgoing = self.client.request(method, &request.url).body(request.body);
        for (name, value) in &request.headers {
            outgoing = outgoing.header(name, value);
        }
        let response = outgoing
            .send()
            .await
            .map_err(|error| EgressError::Transport(error.to_string()))?;
        let status = response.status().as_u16();
        let headers: BTreeMap<String, String> = response
            .headers()
            .iter()
            .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_string())))
            .collect();
        let body = response
            .bytes()
            .await
            .map_err(|error| EgressError::Transport(error.to_string()))?
            .to_vec();
        Ok(EgressResponse {
            status,
            headers,
            body,
        })
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        Err(EgressError::Transport("no streaming".into()))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn asks_pi_poppy_company_about_an_order() {
    let (Ok(pi_poppy), Ok(_)) = (
        std::env::var("PI_POPPY_DIR"),
        std::env::var("OPENAI_API_KEY"),
    ) else {
        return;
    };
    let key = PoppyKey::generate().unwrap();
    let pkcs8 = key.pkcs8().to_vec();

    // The agent's identity documents, served where the company's HTTPS front
    // for `agent.poppy.localhost` forwards. The agent itself is built once
    // the company's port is known; it signs with the same key.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let identity = format!("http://{}", listener.local_addr().unwrap());
    let routes = identity_routes(&PoppyPersonalAgent::new(
        format!("https://{AGENT_HOST}"),
        key,
    ));
    tokio::spawn(async move { axum::serve(listener, routes).await });

    let driver = format!(
        "{}/../../scripts/poppy-interop/company.ts",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut company = tokio::process::Command::new("node")
        .args(["--experimental-strip-types", "--no-warnings", &driver])
        .env("PI_POPPY_DIR", &pi_poppy)
        .env("AGENT_SERVER", &identity)
        .env("COMPANY_DOMAIN", COMPANY_HOST)
        .env("AGENT_HOST", AGENT_HOST)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("run node");
    let mut lines = BufReader::new(company.stdout.take().unwrap()).lines();
    let ready: Value = loop {
        let line = lines
            .next_line()
            .await
            .unwrap()
            .expect("the company driver exited before it was ready");
        if let Some(json) = line.strip_prefix("COMPANY ") {
            break serde_json::from_str(json).unwrap();
        }
    };
    let port = ready["port"].as_u64().unwrap() as u16;
    let ca = std::fs::read(ready["ca_file"].as_str().unwrap()).unwrap();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .add_root_certificate(reqwest::Certificate::from_pem(&ca).unwrap())
        .resolve(
            COMPANY_HOST,
            std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        )
        .build()
        .unwrap();

    let poppy = PoppyPersonalAgent::with_egress(
        format!("https://{AGENT_HOST}"),
        PoppyKey::from_pkcs8(&pkcs8).unwrap(),
        std::sync::Arc::new(ToTestCompany { client }),
    );
    let state = std::env::temp_dir().join(format!("everruns-poppy-test-{}", std::process::id()));
    let session =
        Engine::new().create(personal_agent(openai_model().unwrap(), poppy, &state).unwrap());
    let domain = format!("{COMPANY_HOST}:{port}");
    let turn = session
        .run(
            format!(
                "Ask {domain} whether my order ord_7Hk2 has shipped, and tell me what they said."
            )
            .as_str(),
        )
        .await
        .unwrap();
    let calls = poppy_calls(&session).await.unwrap();
    eprintln!("tool calls: {calls:?}\nanswer: {}", turn.response);

    assert!(
        calls
            .iter()
            .any(|(name, arguments)| name == "poppy_send" && arguments["company"] == domain),
        "the agent should message the company: {calls:?}"
    );
    // pi-poppy answers order questions from a signed-out Session with "I need
    // you to sign in before I can see that order."; the agent relays it.
    let answer = turn.response.to_lowercase();
    assert!(
        answer.contains("sign in") || answer.contains("sign-in"),
        "{}",
        turn.response
    );

    drop(company.stdin.take());
    let _ = company.wait().await;
}
