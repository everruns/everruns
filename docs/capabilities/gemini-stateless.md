---
title: Gemini Stateless
description: Opt out of Gemini Interactions conversation storage while preserving tool calls and signed reasoning through full transcript replay.
appliesTo: [framework, platform, cloud]
---

Add `gemini_stateless` to an agent or harness to send `store: false` on Google
Gemini Interactions requests. Everruns ignores prior interaction IDs and sends
the full transcript, including thought signatures. Other providers are unchanged.
This controls conversation storage, not provider logging, training, or explicit
cached-content resources. The generateContent fallback already replays history.

```json
{"ref": "gemini_stateless"}
```

In Framework code with the default features:

```rust
use everruns::Agent;
let builder = Agent::builder().capability("gemini_stateless");
```

Low-level driver callers can set `LlmCallConfig.driver_options["gemini/interactions"]`
to `{"store": false}`. The typed `GeminiInteractionOptions` is available from
`everruns_drivers::gemini::options` with the `gemini` feature. Without the option,
storage remains enabled and continuation sends only new messages. Invalid option
values fail before an Interactions request is sent.

Completed generations with captured requests record `conversation_store` and
`conversation_continuation` under `request_options.provider_options.gemini`, along with
the actual streaming setting. See [Google Gemini](/providers/gemini/).
