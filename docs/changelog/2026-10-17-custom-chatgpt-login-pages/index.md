---
title: Framework hosts can brand ChatGPT login completion
description: The ChatGPT Plan loopback helper now accepts custom HTML for its successful callback page while retaining its existing plain-text default.
date: 2026-10-17
kind: feature
significance: minor
products: [framework]
release: "0.49.0"
surfaces: [code]
links:
  docs: [/features/chatgpt/]
---

Framework hosts can show their own branded completion page after a successful
ChatGPT Plan login. The loopback helper keeps its existing plain-text response
unless the host supplies HTML, so current integrations do not need to change.

```rust
use everruns_drivers::chatgpt::login::LoginAttempt;

fn brand_login(attempt: LoginAttempt, html: String) -> LoginAttempt {
    attempt.with_success_page_html(html)
}
```

The custom page is returned only after the callback path and state match the
pending attempt. See the [ChatGPT Plan framework guide](/features/chatgpt/) for
the loopback sign-in flow.
