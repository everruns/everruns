//! Python as the agent's execution interface, with durable private state.
//! Set OPENAI_API_KEY, EVERRUNS_MONTY_BIN and EVERRUNS_MONTY_STATE_DIR.
//! cargo run -p everruns --features monty,openai --example python_code_mode
use everruns::{Agent, Engine, OpenAI, PythonCodeMode};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .provider(OpenAI::from_env()?)
        .model("gpt-6.2-sol")
        .instructions("Use Python to perform this task. Discover workspace tools with tools('file'). Use tool(name, arguments) for files, and check its error field. Resume a suspended program with its resume token instead of rerunning it.")
        .capability(PythonCodeMode)
        .capability("session_file_system")
        .build()?;
    let turn = Engine::new().create(agent)
        .run("Using one Python program, calculate the squares of 1 through 12, write them as JSON to /workspace/squares.json using the workspace tool, and report their sum.")
        .await?;
    if !turn.success {
        return Err(format!("Agent failed: {:?}", turn.error).into());
    }
    println!("{}", turn.response);
    Ok(())
}
