//! ag-ui: a serve app streamed to CopilotKit or any AG-UI 1.0 client.
//!
//! The `ag-ui` feature serves `POST /v1/channels/assistant/ag-ui`; the `deploy`
//! tool needs approval, which an AG-UI client sees as an interrupt.
//!
//! ```sh
//! cargo run -p serve-example-ag-ui              # dev server on :3000
//! cargo run -p serve-example-ag-ui -- eval      # run evals/ in-process
//! cd examples/serve/ag-ui/client && npm install && npm start
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
