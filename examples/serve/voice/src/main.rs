//! voice: a serve app people can call from the browser.
//!
//! The `voice` feature gives every top-level agent a voice channel: open
//! `http://127.0.0.1:3000/v1/channels/concierge/voice` and select Start call.
//! The agent is the same one typed messages reach; `[voice]` in `serve.toml`
//! sets how calls sound.
//!
//! ```sh
//! OPENAI_API_KEY=… cargo run -p serve-example-voice   # dev server on :3000
//! cargo run -p serve-example-voice -- eval            # run evals/ in-process
//! ```

mod agent;
#[path = "../evals/mod.rs"]
mod evals;
mod tools;

serve::assets!();

#[tokio::main]
async fn main() -> serve::Result {
    serve::start(serve::App::builder().discover().build()).await
}
