//! a2a: two agents talking over A2A.
//!
//! This binary is a serve app whose `researcher` agent is served over A2A
//! 1.0 (the `a2a` feature): `POST /v1/e/researcher/a2a`, with its Agent Card
//! at `/v1/e/researcher/a2a/.well-known/agent-card.json`. The `writer` binary
//! is an `everruns` agent that delegates research to it.
//!
//! ```sh
//! cargo run -p serve-example-a2a --bin researcher            # dev server on :3000
//! cargo run -p serve-example-a2a --bin writer -- "tide pools" # in another shell
//! cargo run -p serve-example-a2a --bin researcher -- eval    # run evals/ in-process
//! ```

mod agent;
#[path = "../evals/mod.rs"]
mod evals;

serve::assets!();

#[tokio::main]
async fn main() -> serve::Result {
    serve::start(serve::App::builder().discover().build()).await
}
