# Hosted Work Chat

Standalone Next.js browser host for published `work_chat` channels. It serves
one channel per `<assigned-host>.<CHAT_DOMAIN>`. The host is a short label
assigned when the channel is created. It is not the channel id. The console
and this host run independently. Several channels may target the same Agent.

The publisher chooses `multi_chat` or `single_with_threads`. Single mode
requires effective coordinator thread tools; inherited harness configuration
counts, worker-only coordination does not. Workspace tools enable the workspace
panel. Existing questions, tool approvals, workspace permissions and budgets
apply. Consumers are virtual users, separate from console members.

## Deploy

Run the server migrations first. Build this directory with `pnpm install
--frozen-lockfile && pnpm build`, or build its Dockerfile from the repository
root. Keep the sibling `apps/ui/generative-ui` package and the UI security
backport scripts in the checkout: both chat hosts use that shared renderer.
Configure these runtime variables without exposing them to the browser:

| Variable                  | Purpose                                                             |
| ------------------------- | ------------------------------------------------------------------- |
| `CHAT_DOMAIN`             | Hosted content domain, independently isolated from console cookies  |
| `CHAT_API_URL`            | Internal server base, including `/api` if deployed with that prefix |
| `CHAT_OIDC_ISSUER`        | HTTPS OIDC issuer, matching the server's default Work Chat issuer   |
| `CHAT_OIDC_CLIENT_ID`     | Dedicated relying party, also set on the API server                 |
| `CHAT_OIDC_CLIENT_SECRET` | Relying party secret                                                |
| `CHAT_SESSION_KEY`        | Dedicated random 32-byte key encoded as 64 hexadecimal characters   |
| `CHAT_DATABASE_URL`       | PostgreSQL account restricted to `hosted_chat_credentials`          |

Set `CHAT_DOMAIN`, `CHAT_OIDC_ISSUER` and `CHAT_OIDC_CLIENT_ID` on the API server.
Register exactly `https://login.<CHAT_DOMAIN>/auth/callback` with the provider.
The broker uses authorization code, PKCE, nonce and offline refresh. The initial
browser host supports RS256 OIDC; core channel authentication supports additional
provider methods independently. Provider credentials remain encrypted server-side.

Configure wildcard DNS and HTTPS for the content domain. The edge must preserve
the original Host, replace forwarded peer headers, limit request bodies to 6 MiB
and support streaming. Keep the host's internal port private. For on-demand TLS,
use `/host/verify?domain=<requested-host>` as the certificate admission endpoint;
it accepts live Work Chat channels and the fixed broker only. `/health` checks
credential-table readiness. Do not grant the browser host platform database or
management API credentials.

The dedicated database role needs SELECT, INSERT, UPDATE and DELETE on the
credential table only. API migrations own the schema. Configure the role through
your deployment's credential provisioning, never through a conversation.

## Development and verification

`pnpm dev` starts the browser host. Real login requires HTTPS and the registered
broker callback, including in a local environment. Keep `AUTH_MODE=none` limited
to local API development; it deliberately disables console authorization.

`pnpm test` checks hostname/origin and encrypted credential boundaries.
`pnpm e2e` checks both interfaces and mobile rendering using explicit fixtures.
Those rendering tests do not assert real identity-provider or Agent execution.
The server's Work Chat domain tests cover publication, concurrent main creation,
consumer isolation and bounded thread delegation.

After `pnpm build`,
`TEST_CHAT_DATABASE_URL=<disposable-local-connection> pnpm test:broker` runs a
real HTTPS browser, PostgreSQL handoff/session storage and protocol fixtures for
the OIDC provider and execution admission. It needs OpenSSL and permission to
bind localhost port 443. It verifies replay rejection, encryption, host cookie
scope, copied-cookie/sibling-origin rejection and server-side logout revocation.
The test creates and removes its own random database schema.

When embedding this app in a larger npm workspace, set the build-only
`CHAT_OUTPUT_TRACING_ROOT` to that workspace's absolute root. The standalone
server path then includes the app's path relative to that root. This includes
hoisted dependencies without importing the console app at runtime.

Login admission defaults to 120 requests per peer per ten minutes. Set the
optional `CHAT_LOGIN_LIMIT_PER_10_MINUTES` (1–10,000) for your deployment's NAT
and traffic profile. Limited requests return HTTP 429 with `Retry-After`.

## Conversation features

Work Chat includes structured OpenUI/A2UI rendering, reply work logs, task cards,
multiple attachments, Markdown/image/CSV/HTML artifact previews, downloads and
revision-checked source editing. Conversation actions support rename, pin,
archive/reopen and explicit revocable read-only links. Messages use the model
configured on the Agent. No separate feature flag enables these UI features.

Public links expose projected messages and widgets only. They do not reuse a
consumer credential or expose workspace files. Preview frames have an opaque
sandbox with scripts and network access disabled.

## Connection recovery

Drafts and uncertain message requests are retained in this tab's session storage,
scoped to a random namespace issued for the browser login and the conversation.
The namespace is not a login credential. Retry uses the original request body
and Agent API idempotency key; reload never resends an old instruction. Retry is
blocked before the server's one-day retention expires. Discard removes a local
copy without cancelling accepted work. Logout clears local recovery.

Browser quota or disabled storage produces a warning; keep the tab open and
copy unsent text before leaving. Closing the tab or clearing site data can remove
recovery. An offline application shell and offline inference are not provided.
