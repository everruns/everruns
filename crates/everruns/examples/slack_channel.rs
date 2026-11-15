//! Signed Slack Events/Interactivity endpoint with a real agent.
//! Requires OPENAI_API_KEY, SLACK_BOT_TOKEN, SLACK_SIGNING_SECRET and SLACK_TEAM_ID.
//! cargo run -p everruns --features openai,channels --example slack_channel
//! Point Slack's Events API and Interactivity URLs at your HTTPS tunnel's /slack route.
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use everruns::{
    Agent, Engine, Model, OpenAI,
    channels::{Channel, ChannelRequest, Channels, Slack, SlackAdmission},
};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = Agent::builder()
        .instructions(concat!(
            "Help the person in this Slack conversation. Be concise. ",
            "For actions needing consent, ask with request_approval, ",
            "then stop and wait for their response.",
        ))
        .model(Model::new("gpt-6.1-sol", OpenAI::from_env()?))
        .capability("soft_approval")
        .build()?;
    let engine = Engine::new();
    let slack = Slack::new(
        std::env::var("SLACK_BOT_TOKEN")?,
        std::env::var("SLACK_SIGNING_SECRET")?,
    )
    .workspace(std::env::var("SLACK_TEAM_ID")?)
    .agent_pane()
    .task_progress()
    .approval_buttons()
    .thread_history(30)
    .participation(SlackAdmission::Direct);
    let channels = Channels::builder(&engine)
        .channel("slack", Channel::slack(slack), agent)
        .build();
    let router = Router::new()
        .route("/slack", post(handle))
        .with_state(channels);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3137").await?;
    println!("Slack webhook on http://127.0.0.1:3137/slack; use HTTPS ingress for Slack.");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    engine.shutdown(std::time::Duration::from_secs(10)).await;
    Ok(())
}
async fn handle(
    State(channels): State<Channels>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, Json<serde_json::Value>) {
    let request = ChannelRequest {
        path: String::new(),
        headers: headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.to_string(), v.to_string()))
            })
            .collect(),
        body: body.to_vec(),
    };
    let response = channels.handle("slack", &request).await;
    (
        StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(response.body),
    )
}
