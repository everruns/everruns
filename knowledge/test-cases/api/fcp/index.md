# FCP (API)

* [TC001: FCP handshake (GET) returns Markdown](TC001_handshake_returns_markdown.md) - Verify that a `GET` on a published app's FCP endpoint returns the auto-generated Markdown handshake describing how to talk to the endpoint.
* [TC002: FCP POST plain text round-trip](TC002_post_text_returns_reply.md) - Verify that posting plain text to the FCP endpoint produces a Markdown reply from the agent, sets the `fcp_session` cookie, and that a follow-up POST with that cookie resumes the same session.
* [TC003: FCP token authentication](TC003_token_authentication.md) - Verify that an FCP channel with a configured shared token rejects requests without the token and accepts requests bearing the correct token via either `Authorization: Bearer` or `X-Everruns-FCP-Token`.
* [TC004: FCP rate limiting and 404 sanitization](TC004_rate_limit_and_404.md) - Verify two isolation invariants of the FCP endpoint: 1.
