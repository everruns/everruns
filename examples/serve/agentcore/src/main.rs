//! agentcore: a serve app packaged for Amazon Bedrock AgentCore Runtime.
//!
//! ```sh
//! cargo run -p serve-example-agentcore -- agentcore --dev   # AgentCore contract on :8080, offline
//! cargo run -p serve-example-agentcore -- dev               # serve's dev server on :3000
//! cargo run -p serve-example-agentcore -- eval              # run evals/ in-process
//! ```
//!
//! With no command, which is what AgentCore runs, the binary serves the
//! AgentCore contract in serve's `start` mode.

mod agent;
#[path = "../evals/mod.rs"]
mod evals;
mod tools;

serve::assets!();

#[tokio::main]
async fn main() -> serve::Result {
    serve_agentcore::start(serve::App::builder().discover().build()).await
}
