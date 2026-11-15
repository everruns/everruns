---
title: MCP authentication
description: Bind application credentials or an OAuth token store to an exact MCP endpoint, with shared PKCE login and token refresh.
---

Enable `mcp-auth` to use `everruns::mcp::auth`. The MCP transport, OAuth discovery, PKCE exchange, resource/issuer checks, refresh and token rotation are shared with Server. Your application owns login navigation, the callback route, and secret storage. Server uses the same protocol client with its organization-scoped network policy, encrypted records and cross-replica refresh locks; these host adapters remain separate. See the [implementation-sharing matrix](/framework/service-integrations/#shared-implementations-and-host-boundaries).

## Bearer credentials

```rust
use everruns::{Agent, McpServer, Model, mcp::auth::McpAuth};

let url = "https://mcp.example.com/mcp";
let auth = McpAuth::new().bearer("docs", url, std::env::var("DOCS_MCP_TOKEN")?);
let agent = Agent::builder()
    .instructions("Consult the docs server before answering.")
    .model(Model::new("gpt-6-luna", everruns::OpenAI::from_env()?))
    .mcp_server(McpServer::http("docs", url))
    .mcp_auth(auth)
    .build()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Names and endpoints must match at build time and again when credentials are requested. Replacing a live MCP endpoint cannot redirect its existing credential. Unbound servers get no credential from this value. For rotating keys, implement `Credentials::authorization` and use `.credentials(name, url, provider)`. Acquisition happens for each operation; provider errors are sanitized.

## OAuth

`OAuthLogin::begin` discovers metadata and prepares a PKCE login. Use `begin_with_client` when the authorization server requires a pre-registered public client. Its redirect URI must be permitted by that server.

Open `authorization_url()` in the user's browser, retain the login in the same trusted application, and give `complete(code, state, issuer)` the callback's values. `parse_callback_query` rejects duplicate parameters and OAuth error callbacks. Completion consumes the prepared login and validates state, issuer, PKCE and resource binding.

Save the returned `OAuthTokens` into an application-owned secret store implementing `TokenStore`. Bind it with `McpAuth::oauth(name, url, store)`. Refreshed and rotated grants are saved through the same store. A production store must protect access/refresh tokens and coordinate concurrent writers; store serialization is never agent configuration or event history.

`MemoryTokenStore` is explicitly process-local. Outstanding logins and grants do not survive a process restart unless your application persists them securely.

The [complete OAuth example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/mcp_oauth.rs) performs login, accepts a callback URL on stdin, stores the grant in memory, and runs a real agent against the authenticated MCP server:

```bash
cargo run -p everruns --features mcp-auth,openai --example mcp_oauth -- \
  https://MCP_HOST/mcp http://localhost:8080/callback
```

Treat login/callback URLs as credentials. Do not place them in logs, chat messages, or analytics.

Discovery tries the MCP endpoint's path-specific protected-resource document, then the origin's root document. Authorization-server discovery supports issuer paths and OpenID fallbacks, and rejects an advertised issuer that differs from the selected issuer. Callback issuer mismatches are rejected before code exchange. Shared HTTP client errors expose recognized OAuth codes such as `invalid_grant`, while provider descriptions, response bodies and transport URLs are omitted.

Server presets with manually configured OAuth endpoints must also configure the issuer when the provider returns callback `iss`. Automatic discovery records it. A callback issuer without a trusted configured or discovered issuer is rejected; signing in again without correcting the preset cannot establish that trust. Providers that omit `iss` remain supported.
