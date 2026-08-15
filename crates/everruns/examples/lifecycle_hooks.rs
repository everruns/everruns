//! Run typed application code at agent, turn, tool, and completion boundaries.
//!
//! Set `OPENAI_API_KEY`, then run:
//!
//! ```text
//! cargo run -p everruns --features openai --example lifecycle_hooks
//! ```
//!
//! Lifecycle hooks are awaited extension points. Use `Session::events()`
//! instead when code only needs a non-blocking observation stream.

use everruns::{Agent, InMemoryEngine, OpenAI};

/// Return a deterministic service status so the model has a tool to call.
#[everruns::tool]
async fn service_status(service: String) -> Result<String, String> {
    Ok(format!("{service} is healthy"))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .name("lifecycle-demo")
        .instructions("Always call service_status before answering a status question.")
        .provider(OpenAI::from_env()?)
        .model("gpt-5.6-terra")
        .tool(service_status())
        .on_agent_start(|context| async move {
            println!(
                "agent {} started session {}",
                context.agent_name, context.session_id
            );
        })
        .on_turn_start(|context| async move {
            println!("turn input role: {:?}", context.input.role);
        })
        .on_tool_start(|context| async move {
            println!("calling {} with {}", context.tool_name, context.arguments);
        })
        .on_tool_end(|context| async move {
            println!(
                "tool {} finished (success={})",
                context.tool_name,
                context.success()
            );
        })
        .on_completion(|context| async move {
            println!("turn completed: {:?}", context.turn.stop_reason);
        })
        .build()?;

    let turn = InMemoryEngine::new()
        .create(agent)
        .send_and_wait("Is the payments service healthy?")
        .await?;
    println!("response: {}", turn.response);

    for failure in &turn.hook_failures {
        eprintln!("hook warning: {failure}");
    }

    Ok(())
}
