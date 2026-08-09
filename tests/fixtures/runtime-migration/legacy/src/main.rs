use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_runtime::InProcessRuntimeBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = InProcessRuntimeBuilder::new()
        .llm_sim(LlmSimConfig::fixed("4"))
        .single_session(|session| {
            session
                .harness("math", "Answer arithmetic questions.")
                .agent("math-agent", "Return only the answer.")
        })
        .build()
        .await?;
    let session_id = runtime.default_session_id().expect("single session id");
    let turn = runtime.run_text_turn(session_id, "What is 2 + 2?").await?;

    assert!(turn.success);
    assert_eq!(turn.response, "4");
    println!("{}", turn.response);
    Ok(())
}
