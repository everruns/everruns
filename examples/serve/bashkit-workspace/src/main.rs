//! A persistent session workspace served over HTTP, using Bashkit's virtual shell.

mod agent;

serve::assets!();

#[tokio::main]
async fn main() -> serve::Result {
    serve::start(serve::App::builder().discover().build()).await
}
