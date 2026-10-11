---
title: Host Everruns serve apps on Amazon Bedrock AgentCore Runtime
description: The experimental everruns-serve-agentcore crate runs a serve app on AgentCore Runtime with AG-UI streaming, a WebSocket transport and Bedrock models.
date: 2026-10-02
kind: feature
significance: notable
products: [framework]
surfaces: [code, operations]
release: "0.34.1"
links:
  docs: [/framework/serve-agentcore/]
  prs: [everruns/everruns#3990, everruns/everruns#4012]
---

The new `everruns-serve-agentcore` crate hosts a serve app on Amazon Bedrock
AgentCore Runtime. It answers `GET /ping` and `POST /invocations`, which take
an AG-UI `RunAgentInput` or `{"prompt": ...}` and stream AG-UI events, plus a
`/ws` WebSocket transport, with serve's `/v1` API alongside.

The session log and agent workspace live on AgentCore session storage. Bedrock
models use the runtime's execution role when `AWS_REGION` is set, and parked
approvals and `ask_user` questions survive a restart.

See [Serve on AgentCore](/framework/serve-agentcore/). PRs [#3990](https://github.com/everruns/everruns/pull/3990),
[#4012](https://github.com/everruns/everruns/pull/4012).
