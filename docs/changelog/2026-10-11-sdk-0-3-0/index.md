---
title: "SDK 0.3.0 release"
description: "SDKs default to the Everruns Cloud API URL, record why a change was made, and add an AgentClient for calling one agent through the Agent API."
date: 2026-10-11
kind: release
significance: notable
products: [sdk]
release: "0.3.0"
---

## Highlights

- New `AgentClient` in the Python, TypeScript, and Rust SDKs for calling one agent through the Agent Execution API with an agent key (`EVERRUNS_AGENT_URL` / `EVERRUNS_AGENT_KEY`). It covers `run()`, sessions, events and SSE streaming, cancel, questions, tool approvals, runtime tokens, `for_end_user`, and idempotency keys. The management client is now deprecated in favor of `AgentClient`.
- Change reasons: all SDKs gain `with_reason` / `withReason`, a derived client that sends the `Everruns-Change-Reason` header so the server records the reason in entity change history.
- The default base URL is now Everruns Cloud (`https://app.everruns.com/api`) instead of a placeholder, so a client created with only `EVERRUNS_API_KEY` works. `EVERRUNS_API_URL` still overrides it for self-hosted.
- Docs: how to connect an AG-UI client to an Everruns endpoint.
- OpenAPI examples no longer name the sunset Claude Sonnet 4.5.

## In the changelog

- [Everruns SDKs default to the Everruns Cloud API URL](/changelog/2026-10-02-sdk-default-cloud-base-url/)
- [Everruns SDKs record why a change was made](/changelog/2026-10-08-sdk-change-reasons/)
- [Everruns SDKs add AgentClient to call one agent with a key](/changelog/2026-10-10-sdk-agent-client/)

See the [GitHub release](https://github.com/everruns/sdk/releases/tag/v0.3.0) and the [full release notes](https://github.com/everruns/sdk/blob/main/CHANGELOG.md#030---2026-10-11) for every pull request in this version.
