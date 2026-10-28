//! Local HTTP transport/parsing cost: fixed 256 KiB answer, no model generation.
//! NATIVE_BENCH_OUTPUT=/tmp/native.jsonl cargo bench -p everruns-drivers --features openai --bench native_transport
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
use everruns_contracts::{BearerAuth, LlmCallConfig, Provider};
use everruns_drivers::openai::OpenAIChatDriver;
use serde_json::{Value, json};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
#[path = "../tests/support/native_response.rs"]
mod native_response;
use native_response::response_resource;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    use std::{
        io::Write,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Instant,
    };
    let mut file = std::env::var("NATIVE_BENCH_OUTPUT").ok().map(|output| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(output)
            .unwrap()
    });
    let server = MockServer::start().await;
    let json_body = response_resource(&"x".repeat(256 * 1024)).to_string();
    let mut sse = format!(
        "data: {}\n\n",
        json!({"type":"response.output_item.done", "sequence_number":0, "output_index":0, "item":response_resource("")["output"][0]})
    );
    for _ in 0..256 {
        sse.push_str(&format!(
            "data: {}\n\n",
            json!({"type":"response.output_text.delta","delta":"x".repeat(1024)})
        ));
    }
    // The complete resource is authoritative on SSE too.
    sse.push_str(&format!("data: {}\n\n",json!({"type":"response.completed","response":serde_json::from_str::<Value>(&json_body).unwrap()})));
    let bytes = Arc::new(AtomicUsize::new(0));
    let measured = bytes.clone();
    Mock::given(method("POST"))
        .respond_with(move |request: &wiremock::Request| {
            let stream = request.body_json::<Value>().unwrap()["stream"]
                .as_bool()
                .unwrap();
            let body = if stream { &sse } else { &json_body };
            measured.fetch_add(body.len(), Ordering::Relaxed);
            ResponseTemplate::new(200)
                .insert_header(
                    "content-type",
                    if stream {
                        "text/event-stream"
                    } else {
                        "application/json"
                    },
                )
                .set_body_string(body.clone())
        })
        .mount(&server)
        .await;
    let provider = Provider::new("bench", OpenAIChatDriver::new())
        .base_url(server.uri())
        .auth(BearerAuth::new("synthetic-key"));
    let config = LlmCallConfig::new("gpt-6.1-sol");
    // Alternating modes keeps cache/thermal effects from favoring one path.
    for sample in 0..22 {
        for native in [false, true] {
            let before = bytes.load(Ordering::Relaxed);
            let started = Instant::now();
            let answer = if native {
                provider
                    .chat_completion_non_streaming(vec![], &config)
                    .await
            } else {
                provider.chat_completion(vec![], &config).await
            }
            .unwrap();
            assert_eq!(answer.text.len(), 256 * 1024);
            assert_eq!(answer.tool_calls.unwrap().len(), 1);
            assert_eq!(answer.reasoning.len(), 1);
            let row = json!({"sample":sample,"native":native,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"response_bytes":bytes.load(Ordering::Relaxed)-before});
            if sample >= 2 {
                if let Some(file) = file.as_mut() {
                    writeln!(file, "{row}").unwrap();
                }
                println!("{row}");
            }
        }
    }
}
