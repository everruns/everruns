//! revenue-analyst: the full serve layout, runnable offline.
//!
//! ```sh
//! cargo run -p serve-example-revenue-analyst              # dev server on :3000
//! cargo run -p serve-example-revenue-analyst -- eval      # evals/ in-process
//! cargo run -p serve-example-revenue-analyst -- manifest  # the host contract
//! cargo run -p serve-example-revenue-analyst -- deploy    # what a host would provision
//! ```

mod agent;
mod channels;
mod connections;
#[path = "../evals/mod.rs"]
mod evals;
mod schedules;
mod subagents;
mod tools;

serve::assets!();

#[tokio::main]
async fn main() -> serve::Result {
    serve::start(serve::App::builder().discover().build()).await
}
