//! hello: the smallest serve app.
//!
//! ```sh
//! cargo run -p serve-example-hello              # dev server on :3000
//! cargo run -p serve-example-hello -- eval      # run evals/ in-process
//! cargo run -p serve-example-hello -- manifest  # the host contract
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
