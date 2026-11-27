//! Real Slack-native tools. Requires SLACK_BOT_TOKEN, SLACK_CHANNEL_ID and SLACK_THREAD_TS.
//! cargo run -p everruns --features openai,slack-actions --example slack_actions
use everruns::{Agent, Engine, Model, OpenAI, slack::SlackActions};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let channel = std::env::var("SLACK_CHANNEL_ID")?;
    let thread = std::env::var("SLACK_THREAD_TS")?;
    let actions = SlackActions::new(
        std::env::var("SLACK_BOT_TOKEN")?,
        channel.clone(),
        thread.clone(),
    );
    let agent = Agent::builder()
        .instructions(format!(
            "The application has authorized and bound Slack tools to channel {channel}, \
             thread root {thread}. Use Slack actions only in that conversation. \
             Add an eyes reaction to the supplied message and upload a short text file \
             that says this Framework bot is connected. \
             Report success only after checking tool results.",
        ))
        .model(Model::new("gpt-6.2-sol", OpenAI::from_env()?))
        .slack_actions(actions)
        .build()?;
    let session = Engine::new().create(agent);
    let prompt = format!(
        "Use channel {channel}, thread {thread}. React to the thread root \
         and upload framework-hello.txt into that thread."
    );
    let turn = session.run(prompt).await?;
    if !turn.success {
        return Err(format!("Agent failed: {:?}", turn.error).into());
    }
    println!("{}", turn.response);
    Ok(())
}
