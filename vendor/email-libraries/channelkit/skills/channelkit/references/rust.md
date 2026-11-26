# Rust and provider integration

## Versioned Channelkit guides

Each crate's Rustdoc includes a `guide` module. The crate bundles also contain
a canonical `docs/` guide for that provider; a source checkout is
not required. Use the guide for the application's resolved Channelkit version.

## Compose with the application's runner

Pass configured adapters into the coordinator. This example assumes the host has
already resolved credentials and constructed both adapters; keep only the providers
needed by the application:

```rust
use channelkit::{Channels, MessageContext};
use channelkit_slack::Slack;
use channelkit_whatsapp::WhatsApp;

async fn reply(ctx: MessageContext) -> channelkit::Result<()> {
    ctx.thread().subscribe().await?;
    ctx.thread()
        .post(format!("echo: {}", ctx.message().text))
        .await?;
    Ok(())
}

pub fn channels(slack: Slack, whatsapp: WhatsApp) -> channelkit::Result<Channels> {
    Channels::builder()
        .adapter(slack)
        .adapter(whatsapp)
        .on_new_mention(reply)
        .on_subscribed_message(reply)
        .on_direct_message(reply)
        .build()
}
```

Replace the echo with the application's agent runner. `.data(app_data)` changes the
handler context to `MessageContext<AppData>`; register data before handlers. Use
`.on_message(...)` for a catch-all integration, or adapters directly when the host
already owns dispatch. `channels!` and `card!` are optional conveniences.

Wire existing HTTP routes to verification over the original body bytes and headers.
`Channels::receive` returns an `Ingress`; it starts no listener, handler, or durable
handoff. Apply the host's acceptance policy before returning `Ingress.response` and
dispatch accepted events explicitly. Provider authenticity does not authorize agent
tools: bind actors, installations, destinations, and control actions in the host.

## Provider configuration

Use the application's secret manager/environment or explicit adapter constructors.
`from_env` reads these variables; it does not provision accounts or install an app:

| Adapter | Required environment variables |
| --- | --- |
| Slack | `SLACK_BOT_TOKEN`, `SLACK_SIGNING_SECRET`, `SLACK_TEAM_ID` |
| WhatsApp | `WHATSAPP_ACCESS_TOKEN`, `WHATSAPP_APP_SECRET`, `WHATSAPP_PHONE_NUMBER_ID`, `WHATSAPP_VERIFY_TOKEN` |

Give each configured installation a stable host-owned identity. Keep webhook signing
secrets separate from API tokens; WhatsApp GET verification and POST signing use
different secrets. Never deserialize/reserialize a signed body before verification.

For Slack, configure the host's webhook/interactivity routes, install the app in the
intended workspace, and grant scopes for the operations actually used. Posting, history,
reactions, and media need their respective scopes. Native agent streams/controls are
conditional; consult capabilities instead of assuming every installation supports them.

For WhatsApp, use the official Business Cloud API and the intended business phone ID.
Contextual replies check the customer-service window. Approved templates are explicit
operations, never automatic fallback. Edit/delete and native history are unsupported;
streamed output is buffered. API acceptance and later delivered/read events are separate.

Across providers, preserve all confirmed receipt references on partial output. Do not
blindly retry `DeliveryUnknown` or `PartialDelivery`. Reactions/actions are feedback;
they do not themselves approve pending work or authorize cancellation.


## Additional adapters

Use the matching `channelkit-discord`, `channelkit-teams`, `channelkit-github`,
`channelkit-linear`, `channelkit-x`, or `channelkit-web` crate and its packaged guide.
Constructors take explicit host-resolved configuration; only adapters documenting a
`from_env` helper supply one. Do not invent environment variable conventions.

Discord verifies Ed25519 HTTP interactions; ordinary messages arrive through the host's
Gateway session. Use `reply` on the saved command Message to complete a deferred reply.
Keep interaction credentials in a host-provided InteractionStore for restarts.
Teams verifies Bot Connector RS256 tokens against fresh trusted Microsoft keys; bind
the app, tenant and connector service URL. Refresh keys outside request parsing.
GitHub keys distinguish issue comments from PR review threads and namespace their IDs.
Linear distinguishes comment threads from append-only agent sessions. Elicitation emits
an activity; the host persists the pending decision and resumes its own work.
X distinguishes public replies from private peer conversations; it uses user-context
OAuth tokens. Web authenticates ownership through the host and emits to a scoped Sink;
it neither trusts body user IDs nor serves browser connections.

The initial `0.1.0-alpha.6` registry scope excludes Teams and X. Those source-only
adapters have incomplete live messaging setup; do not recommend adding their
crate names from crates.io until a published version exists.
