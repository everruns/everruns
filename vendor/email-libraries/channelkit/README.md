# channelkit

[![MIT](https://img.shields.io/badge/license-MIT-3c9c51)](https://opensource.org/license/mit)
[![Rust 1.95+](https://img.shields.io/badge/rust-1.95%2B-dc6b35)](https://www.rust-lang.org/)

Async Rust messaging for application-owned agents. Channelkit is a library with no
service, database, migrations, or durable worker. Your application owns HTTP,
authorization, agents, persistence, and recovery.

Alpha release candidate. APIs may change; provider-origin inbound and
delivery verification remains incomplete.

```toml
[dependencies]
channelkit = "=0.1.0-alpha.6"
```

[Versioned API and guide](https://docs.rs/channelkit/0.1.0-alpha.6/channelkit/)
· [Packaged guide](docs/getting-started.md)

The core coordinates adapters with `Channels::builder()`, thread subscriptions,
message/reaction/action handlers, typed application data, and optional host state hooks.
Provider crates: `channelkit-slack`, `channelkit-whatsapp`, `channelkit-discord`,
`channelkit-github`, `channelkit-linear`, and `channelkit-web`.

[Agent integration skill](https://docs.rs/crate/channelkit/0.1.0-alpha.6/source/skills/channelkit/SKILL.md)
is included in the package with its references; no maintainer checkout is required.

## Sources of inspiration

[AgentChat](https://github.com/BerriAI/agentchat) and [Chat SDK](https://chat-sdk.dev/)
inform the channel/thread/handler model. [Bashkit](https://github.com/everruns/bashkit)
informs repository, testing, knowledge and release conventions;
[Everruns](https://github.com/everruns/everruns) is the intended first adopter.
See the packaged guide for integration contracts and limitations.
