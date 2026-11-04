//! Event serialization and controlled 2 ms emitter wait; no network/database.
//! STREAMING_BENCH_OUTPUT=/tmp/events.jsonl cargo bench -p everruns-test-support --bench output_streaming
#[path = "../tests/integration/reason_atom/output_streaming_fixture.rs"]
mod fixture;
#[path = "../tests/integration/reason_atom/setup.rs"]
mod setup;
use everruns_contracts::runtime::conversation::Communication;
use everruns_core::EventData;
use fixture::{MeasuredEmitter, run};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    use std::io::Write;
    let mut file = std::env::var("STREAMING_BENCH_OUTPUT").ok().map(|path| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
    });
    for sample in 0..5 {
        for communication in [Communication::Direct, Communication::Explicit] {
            let emitter = MeasuredEmitter {
                delay: Duration::from_millis(2),
                ..Default::default()
            };
            let (result, events, elapsed) = run(
                setup::setup_test_environment().await,
                communication,
                None,
                64,
                Duration::from_millis(4),
                emitter.clone(),
            )
            .await;
            assert_eq!(result.text.len(), 64 * 4096);
            let deltas = events
                .iter()
                .filter(|event| {
                    matches!(
                        event.data,
                        EventData::OutputMessageDelta(_) | EventData::ReasonThinkingDelta(_)
                    )
                })
                .count();
            let row = json!({"sample":sample,"communication":communication,"events":events.len(),"deltas":deltas,"event_bytes":emitter.bytes.load(Ordering::Relaxed),"emitter_wait_ms":emitter.wait_ns.load(Ordering::Relaxed) as f64 / 1e6,"elapsed_ms":elapsed.as_secs_f64()*1000.0,"text_bytes":result.text.len()});
            println!("{row}");
            if let Some(file) = file.as_mut() {
                writeln!(file, "{row}").unwrap();
            }
        }
    }
}
