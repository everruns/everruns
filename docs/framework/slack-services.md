---
title: Slack services
description: Use Slack-native action tools and advanced signed Slack channels in standalone Framework applications.
---

Slack action tools and Slack channels are independently optional. Action tools let an agent act in a conversation your application authorizes. Channels receive signed Slack events and deliver agent replies.

## Native tools

Enable `slack-actions`. Bind credentials and the authorized destination outside serialized agent configuration:

```rust
use everruns::{Agent, Model, slack::SlackActions};

let actions = SlackActions::new(
    std::env::var("SLACK_BOT_TOKEN")?, "C012345", "1730000000.000001",
);
let agent = Agent::builder()
    .instructions("Use Slack tools to help in the authorized conversation.")
    .model(Model::new("gpt-6.2-sol", everruns::OpenAI::from_env()?))
    .slack_actions(actions)
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

This enables `slack_add_reaction`, `slack_update_message`, `slack_lookup_user`, and `slack_upload_file`. The operations and tool schemas are shared with Server. Uploads use Slack's external upload reservation, byte transfer, and completion flow; the retired `files.upload` method is not used.

Channel arguments cannot redirect the bound credential. Uploads must use the configured thread. Message edits/reactions in a thread verify target membership; user lookup returns only display identity fields. Upload URLs are constrained to Slack or an explicitly configured trusted API origin, redirects are refused, and signed upload URLs do not enter error messages.

The [native actions example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/slack_actions.rs) reacts to a thread root and uploads a file through a real agent. Set `OPENAI_API_KEY`, `SLACK_BOT_TOKEN`, `SLACK_CHANNEL_ID` and `SLACK_THREAD_TS`:

```bash
cargo run -p everruns --features openai,slack-actions --example slack_actions
```

## Advanced channels

Enable `channels` and configure a Slack app with event subscriptions, a signing secret, and the scopes required by the enabled operations. The driver verifies Slack's signature over the original body and enforces its five-minute replay window. Empty explicitly configured signing secrets are rejected.

```rust
# fn bind(engine: &everruns::Engine, agent: everruns::Agent) -> Result<(), Box<dyn std::error::Error>> {
use everruns::channels::{Channel, Channels, Slack, SlackAdmission};

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
let channels = Channels::builder(engine)
    .channel("support", Channel::slack(slack), agent)
    .build();
# Ok(())
# }
```

Use `Channel::slack` to select delivery per turn: the Slack agent pane streams replies and shows status/title; ordinary channels receive completed messages. Streaming supports close, cancellation cleanup and replacement when a guardrail retracts text. Task progress edits one message in place.

Approval buttons answer `request_approval` questions in the conversation. They are one-use opaque cards, expire after fifteen minutes, are superseded by new conversation input or another card, and may be answered only by the requesting Slack user in the original channel. They do not grant arbitrary durable tool-approval authority. Outstanding cards are process-local; after a restart, the person can answer in the thread or the application can request another card.

Thread history seeds a new session without starting extra turns, excludes the triggering message/bots, and has a hard bound of 100 messages. Participation runs before session creation: use `SlackAdmission::Direct`, implement `Participation`, or use `SlackRelevance::new(decisions)` for an explicitly selected decision model. Relevance times out after two seconds and declines on errors; direct messages and mentions pass.

Pane stop/title/context events route to the existing conversation. Latest title/view state is available through `Channels::conversation_state`; view data accompanies the next user input as untrusted reference data. Supply `ConversationStore` for durable metadata, `ChannelStore` for durable bindings/pending delivery, and a durable Engine profile for sessions. The defaults are process-local.

Mount `Channels::handle` on your HTTP framework, preserving the raw body and signature headers. Keep intake within Slack's three-second acknowledgement deadline. `Slack::from_env` permits unsigned requests when no signing secret exists for local experiments; production applications should use the explicit constructor and require both secrets.

The [complete HTTP example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/slack_channel.rs) mounts both Events API and Interactivity on `/slack`, requires workspace pinning and verifies raw request signatures:

```bash
cargo run -p everruns --features openai,channels --example slack_channel
```

Set the four variables named at the top of that source file and configure Slack's URLs through an HTTPS ingress. The example uses process-local session/binding state. For a durable Engine, pre-register its `AgentBuild` and use `ChannelsBuilder::channel_registered`; re-register that exact build after restart together with durable channel/conversation stores.
