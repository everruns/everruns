# Getting started

Channelkit coordinates messaging inside your Rust application. No database, server,
or durable worker is included. Your application supplies provider credentials, HTTP
routes, agent code, and any persistence it needs.

## Installation

The initial registry release is `0.1.0-alpha.6`, requiring Rust 1.95 or newer.
Use exact prerelease dependencies while the API is evolving:

```toml
[dependencies]
channelkit = "=0.1.0-alpha.6"
channelkit-slack = "=0.1.0-alpha.6"
channelkit-whatsapp = "=0.1.0-alpha.6"
```

Additional published adapters are `channelkit-discord`, `channelkit-github`,
`channelkit-linear`, and `channelkit-web`. Teams and X remain source-only and are
excluded from this release. Development and CI use Rust 1.97.

This alpha has signed fixtures, real loopback HTTP integration tests, restart/HITL
host examples, and separate library/host line-coverage gates of at least 95%.
Slack's designated live suite covers post/readback/edit, reactions and media;
WhatsApp covers approved-template API acceptance; Discord, GitHub and Linear cover
API acceptance and cleanup. These are not provider-origin inbound, delivered/read,
or deployed-adopter proof. Full v0.1 release requirements remain open. Socket Mode,
OAuth installation helpers, stream rotation, typed-action derive and general progress
rendering are not implemented. Hosts own durable execution and token refresh.

The portable [agent skill](https://docs.rs/crate/channelkit/0.1.0-alpha.6/source/skills/channelkit/SKILL.md)
and its references are included under `skills/channelkit/` in the core crate archive.
They describe integration contracts, without maintainer commands or accounts.

## Ordinary builders and macros

```rust
use channelkit::{Button, Card, card};
let content = card! {
    title: "Choose the next step",
    body: "Summary or full report?",
    actions: [Button::new("summary", "Summary"), Button::new("report", "Full report")],
}?;
assert_eq!(content.actions.len(), 2);
# Ok::<(), channelkit::Error>(())
```

`Channels::builder().adapter(slack).adapter(whatsapp).on_message(handler).build()`
creates an in-process coordinator. It starts no listener or background service.
Register application data before handlers: `.data(app_data)` changes their context
to `MessageContext<AppData>`. `.state(host_state)` is optional; bounded memory state
is the default. `channels! { adapters: [...], data: value, on_message: handler }`
expands to the same builder and returns the same configuration errors.

```compile_fail
// Unknown macro configuration is a compiler error, not an ignored option.
let _ = channelkit::card! { unexpected: "field" };
```

```compile_fail
// Invalid handler/event types are rejected by Rust.
use channelkit::MessageContext;
fn wrong(_: u32) {}
let _: fn(MessageContext) = wrong;
```

## Request lifecycle

A provider adapter or `Channels::receive` verifies raw bytes and returns `Ingress`:
a provider response plus normalized events. It does not acknowledge HTTP or start
handlers. The application first accepts events into its chosen execution policy,
then returns the response, and calls `dispatch` explicitly. A memory handoff is not
crash-safe. Everruns can use its own queue and recovery without another runtime.

Use `.on_new_mention`, `.on_direct_message`, `.on_subscribed_message`, `.on_message`,
`.on_reaction`, `.on_action`, `.on_command`, and `.on_event` for lifecycle/control/status
events. DM handlers take priority, then subscribed threads, then new mentions, then
matching ordinary-message filters, then the default message handler. Subscription
state is checked at dispatch. Only one message handler responds by default.

Filters compose as `.and`, `.or`, or the Rust `!`/`Not` operation. A
`.before_dispatch` hook can authorize an actor after provider verification. Provider
provenance does not itself authorize tools or destinations. `.on_error` observes handler failures with the
original shared error; dispatch also returns it. Ingress, configuration, authorization,
and host state-hook errors are returned directly to the caller. A failed handler is not retried
by Channelkit. Host retries after partial side effects need host-level decisions.

## Messages, threads, and receipts

`ctx.thread().post(content).await` returns a `SentMessage`. Its receipt contains all
provider message references. Use `.like()`, `.react(emoji)`, `.edit(content)`, and
`.delete()` where supported. WhatsApp contextual replies use the original message
and check its reply window; uncontextualized sends leave the provider authoritative.

```rust
use channelkit::{ConversationKey, Provider, split_text};
let key = ConversationKey::new(Provider::Slack, "installation", "channel", Some("1.000".into()))?;
assert_eq!(key.provider, Provider::Slack);
assert_eq!(split_text("🙂αβ", 2)?, vec!["🙂α", "β"]);
# Ok::<(), channelkit::Error>(())
```

Native history reports pagination and completeness. Observed in-process history
is bounded, omits outgoing split/stream output and can omit older content, and always reports `native = false` and
`complete = false`. It must not be described as a remote conversation archive.

Streams have shared byte/chunk/deadline limits, including empty chunks and stalled
sources. Control events retain optional actors; authorize stop requests using the
actor and host-owned session binding.

Cancellation tokens are cooperative. Pass the token to stream operations; native
streams are stopped on errors/cancellation and buffered WhatsApp streams send no partial text. Slack post/edit fallback can retain
visible partial output and returns its confirmed references on failure.
An operation whose acceptance is unknown returns `DeliveryUnknown`. Split sends can
return `PartialDelivery` with confirmed references; never blindly resend them.

## Executable examples

`cargo run -p channelkit-echo -- --offline` runs the same handler against both real
adapter implementations using a deterministic transport. Provider integration tests
run signed ingress through real loopback HTTP. Live commands and their prerequisites
are documented separately. See repository `docs/live-testing.md` and `docs/security.md`.

## Explicit feedback

Inside `.on_reaction` or `.on_action`, project feedback onto a host-correlated sent
reply using `event.feedback_for(&reply_reference)` (actions additionally take the
configured positive/negative wire IDs). `FeedbackEvent` retains the actor and original
reaction/action. Removed reactions and clears remain distinguishable; they are not
new positive/negative votes. No projection grants approval. Message identity checks
provider, installation, channel and message ID, even when Slack omits a root thread.

Long Slack Markdown preserves fenced code blocks when splitting across messages,
including language hints and Unicode text. Inline Markdown still uses the provider's
renderer. Fences over 16 delimiters or 128 header bytes return a typed input error
rather than producing an invalid framed message.

## Sources of inspiration

[AgentChat](https://github.com/BerriAI/agentchat) and [Chat SDK](https://chat-sdk.dev/)
are the main API inspirations: application-owned agents, threads, subscriptions,
handlers, rich replies, reactions and streaming.
[Slack Bolt](https://github.com/slackapi/bolt-js),
[Slack Morphism](https://github.com/abdolence/slack-morphism-rust),
[Poise](https://github.com/serenity-rs/poise),
[PyWa](https://github.com/david-lev/pywa), and
[Axum](https://github.com/tokio-rs/axum) inform provider ergonomics and composition.
[Bashkit](https://github.com/everruns/bashkit) informs testing, repository organization,
knowledge and releases. [Everruns](https://github.com/everruns/everruns) provides
adopter experience; its internal runtime does not define Channelkit's API.
