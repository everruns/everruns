//! A personal agent asks a company about an order over Poppy.
//!
//! ```text
//! OPENAI_API_KEY=... POPPY_AGENT_URL=https://agent.example.com \
//!   cargo run -p everruns-poppy-personal-agent -- shop.example.com
//! ```
//!
//! `POPPY_AGENT_URL` is the public HTTPS address that reaches this process on
//! `POPPY_AGENT_PORT` (default 8787), through a reverse proxy or a tunnel.
//! Companies fetch the agent's client metadata and key from there.

use everruns::Engine;
use everruns::poppy::{PoppyKey, PoppyPersonalAgent};
use everruns_poppy_personal_agent::{identity_routes, openai_model, personal_agent, poppy_calls};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let company = args
        .next()
        .ok_or("usage: everruns-poppy-personal-agent <company-domain> [question]")?;
    let question = args.next().unwrap_or_else(|| {
        format!("Ask {company} whether my order has shipped, and tell me what they said.")
    });
    let base_url = std::env::var("POPPY_AGENT_URL")
        .map_err(|_| "set POPPY_AGENT_URL to the public HTTPS address of this agent")?;
    let port: u16 = std::env::var("POPPY_AGENT_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(8787);

    // A new key per run: companies see a new personal agent each time.
    let poppy = PoppyPersonalAgent::new(&base_url, PoppyKey::generate()?);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let routes = identity_routes(&poppy);
    tokio::spawn(async move { axum::serve(listener, routes).await });
    println!("client_id {}", poppy.client_id());

    let state = std::env::temp_dir().join(format!("everruns-poppy-{}", std::process::id()));
    let session = Engine::new().create(personal_agent(openai_model()?, poppy, &state)?);
    println!("person> {question}");
    let turn = session.run(question.as_str()).await?;
    for (tool, arguments) in poppy_calls(&session).await? {
        println!("  {tool} {arguments}");
    }
    println!("agent> {}", turn.response);
    Ok(())
}
