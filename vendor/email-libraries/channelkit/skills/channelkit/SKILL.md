---
name: channelkit
description: Integrate Channelkit into a Rust application using its official messaging adapters. Use when adding Channelkit adapters, handlers, webhook routes, agent replies, or tests of an existing Channelkit integration.
---
# Channelkit integration

Use Channelkit's guides and APIs for the version used by the consuming application.
See [Rust and provider integration](references/rust.md) for composition and provider
configuration. The skill and its references are independent of a source checkout
and maintainer tooling/accounts.

Channelkit is a library with no service, DB, migrations, or durable worker. The host
owns HTTP routes/acknowledgement, agents, permissions, persistence, and recovery.
Choose `Channels` for in-process handlers or `Adapter` directly when a host already
owns dispatch. Parsing an Ingress does not acknowledge or accept durable work.

Use `.data(...)` before registering typed handlers. State hooks are optional; memory defaults
are bounded/transient. Preserve provider/installation identity. Thread subscriptions
follow message routing; reactions/actions/statuses are separate. Feedback is not approval.
Use explicit Content and capabilities; return Unsupported honestly. Unknown/partial send
acceptance must not be blindly retried. Credentials never enter model input or diagnostics.

Official providers are Slack, WhatsApp, Discord, Microsoft Teams, GitHub, Linear, X,
and Web. Telegram and other Russian messaging
channels are excluded from official adapters, dependencies, examples, and recommendations.

Discord Gateway dispatches require a host-authenticated connection; Teams requires fresh
Microsoft signing keys and pinned connector scope. Web requires a host authenticator
and output sink. Hosts supply OAuth refresh through TokenProvider. Consult each packaged
`guide` for native limits and conversation identities.

For integration boundaries and live proof, read [consumer verification](references/testing.md). Use the application's
own secret configuration and designated live recipients/channels with user authorization;
this skill does not authorize messaging others. Report whether evidence is local,
simulated HTTP, API acceptance, remote readback, or provider-origin inbound traffic.

The initial `0.1.0-alpha.6` registry scope excludes Teams and X. Those source-only
adapters have incomplete live messaging setup; do not recommend adding their
crate names from crates.io until a published version exists.
