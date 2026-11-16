---
type: Design
title: "Host-owned browser sessions"
description: "Browser ownership and transport boundaries shared by Framework and coding hosts."
tags:
  - everruns
  - integrations
  - security
---
# Host-owned browser sessions

## Decision

Keep the existing browser action vocabulary shared by native model adapters,
the Framework and coding hosts. Browser attachment and network authority belong
to trusted host configuration, not tool arguments. The implementation lives
beside the [Browserless browser backend](../../crates/integrations/src/browserless/browser_use/mod.rs);
the [contracts](../../crates/contracts/src/runtime/browser_use.rs) own actions.

A browser running in a sandbox must reach applications from that sandbox's
network. The hosted Browserless request interceptor deliberately fetches from
the control host and blocks WebSockets. It is not interchangeable with this
network contract, and its SSRF controls must not be weakened to support local
application development. A direct host-owned browser therefore requires the
host to accept responsibility for browser network isolation and egress.

## Ownership

A named CLI daemon is not an isolation boundary when several clients attach to
the same CDP browser. Allocate a separate browser context for each runtime
session and validate target membership on every action. Never adopt the
browser's default context, personal profile, or another session's target.
Losing a context is an explicit failure, not permission to recreate it and
repeat a possibly completed submission. Cleanup disposes only owned contexts.

Connection endpoints and authentication headers remain in host memory and are
redacted from debug output. Model-facing state contains page data and opaque
target identity, not transport credentials. Non-loopback connections require
TLS. A private network address alone does not authenticate a browser endpoint.

## Personal browsers are a separate trust boundary

An existing user profile needs explicit lending, visible ownership, revocation
and human handoff. It must not be treated as an isolated sandbox. OpenCode
Browse provides useful extension/relay patterns, but its per-process ownership
and OpenCode-specific session metadata are not Everruns tenant authorization.
Its extension-to-relay remote mode is not a remote MCP endpoint.

Browser handoff must park durable work rather than hold a worker waiting for a
person. Only the host's authenticated user response may resume the handoff;
page content cannot approve it. A reconnect must revalidate the lent target.

The current Framework stdio MCP transport starts a child per RPC. Browse keeps
ownership in a persistent process, so adding its executable as a generic MCP
server alone does not preserve the required session semantics. A later adapter
must retain one authenticated process/owner per session and close it explicitly,
with bounded output, cancellation and no accidental pooling across sessions.
Framework client-tool pending results and explicit resolution provide the
building blocks for durable handoff, but the adapter must split starting a
handoff from observing/completing it rather than hold a long-running MCP call.
This phase does not implement that adapter, extension UI or personal tab lending.

## Recovery and verification

Browser connection recovery is not action recovery. A disconnected form submit
may have completed externally; observe and reconcile rather than retry it.
The tool's conservative side-effect classification remains authoritative.

The qualification matrix requires a local fixture from both Framework and coding host,
isolated sessions, stale refs and foreign targets, redirects and subresources,
WebSocket applications in the browser's network, authenticated remote CDP,
closed contexts and cleanup. Record executed cases separately from this target
matrix. A loopback CDP test alone is not sandbox evidence.

## Reference analysis

The inspected upstream revisions were Browse
`c3b29be5299658281fe4ecdcb926bb7b5a9d6f8c` (MIT) and agent-browser
`53384771fbafc261ba67608ac806dd5aa70b84f0` (Apache-2.0). Browse is a paired
extension and native MCP relay; agent-browser provides CLI/stdio MCP and direct
CDP. Prefer adapters over copying internal APIs, and preserve license notices
when source is reused.
