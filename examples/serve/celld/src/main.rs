//! celld: a serve app packaged as a celld container.
//!
//! ```sh
//! cargo run -p serve-example-celld -- celld --dev   # celld contract on :8080, offline
//! cargo run -p serve-example-celld -- dev           # serve's dev server on :3000
//! ```
//!
//! With no command, which is what the container runs, the binary serves the
//! celld contract in serve's `start` mode. The Durable Object that keeps its
//! state durable is in `worker/`.

mod agent;
mod tools;

serve::assets!();

#[tokio::main]
async fn main() -> serve::Result {
    serve_celld::start(serve::App::builder().discover().build()).await
}
