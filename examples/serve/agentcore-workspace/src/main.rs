//! agentcore-workspace: a serve agent that works in its AgentCore microVM.
//!
//! ```sh
//! cargo run -p serve-example-agentcore-workspace -- agentcore --dev   # :8080, offline
//! cargo run -p serve-example-agentcore-workspace -- eval              # run evals/
//! ```
//!
//! `[sandbox] kind = "microvm"` in `serve.toml` gives the agent a real shell
//! and file tools on AgentCore, rooted at session storage (`/mnt/workspace`).
//! Under `dev` and `eval`, bashkit stands in.

mod agent;
#[path = "../evals/mod.rs"]
mod evals;
mod tools;

serve::assets!();

#[tokio::main]
async fn main() -> serve::Result {
    serve_agentcore::start(serve::App::builder().discover().build()).await
}
