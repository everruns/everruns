---
title: Application services
description: Compose GitHub, authenticated MCP, Slack, data services, native tools, triggers, traces, and evaluations without an Everruns server.
---

Framework applications can bind their own credentials, persistence, and retrieval services to the same operations the Server runs. Enable only the Cargo features you use. Credentials stay in application-owned values and never enter an agent's serialized configuration.

| Priority | Feature | Framework API | Feature flag / guide |
| --- | --- | --- | --- |
| 1 | GitHub operations and rotating application credentials | `.capability(GitHub)` | `github`; [GitHub tools](/framework/github/) |
| 1 | MCP bearer credentials, OAuth login, refresh and rotation | `.mcp_auth(McpAuth)` | `mcp-auth`; [MCP authentication](/framework/mcp-authentication/) |
| 1 | Slack reactions, message edits, user lookup and external file uploads | `.slack_actions(SlackActions)` | `slack-actions`; [Slack](/framework/slack-services/) |
| 1 | Slack agent pane, streaming, status/title, approvals, progress, history and participation | `Channel::slack(Slack)` | `channels`; [Slack](/framework/slack-services/) |
| 2 | Event filters, changes-only checks, durable dedupe/outbox and session routing | `triggers::Dispatcher` | `triggers`; [Event triggers](/framework/event-triggers/) |
| 2 | Named agent coordination | `.member(Agent)` | [Coordinator agents](/framework/coordination/) |
| 2 | Cursor, Browserless and image generation | `.cursor(...)`, `.browserless(...)`, `images` | `cursor`, `browserless`, `images`; [Native services](/framework/native-services/) |
| 3 | Persistent, session-scoped SQL | `.sql_database(SqlDatabase)` | `sql-database`; [Data services](/framework/data-services/) |
| 3 | Named read-only or writable directory memory | `.memory(MemoryDirectory)` | [Data services](/framework/data-services/) |
| 3 | Application knowledge bases and citable retrieval indexes | `.knowledge(Knowledge)` | `knowledge`; [Data services](/framework/data-services/) |
| 3 | E2B sandbox create/list/execute/files/pause/resume/delete | `.e2b(...)` | `e2b`; [Native services](/framework/native-services/) |
| 4 | Shared event-to-turn/step projection | `trace::Trace::project` | [Evaluation and traces](/framework/evaluation-and-traces/) |
| 4 | Shared scorers, bounded evaluation runs and online scoring | `evaluation::Evaluator`, `OnlineScoring` | `evaluation`; [Evaluation and traces](/framework/evaluation-and-traces/) |

These are application execution features. Cloud account selection, integration management screens, managed indexing/sync workers, deployment scheduling, and Server reporting APIs remain host responsibilities. The default Framework dependency graph stays offline.

Each guide links a complete example that uses a real model. For model-free verification, acceptance tests use scripted models against real Engine sessions and mock vendor endpoints:

```bash
cargo test -p everruns --features github,mcp-auth,sql-database,knowledge,slack-actions,channels,triggers,evaluation --test service_parity --test github_framework
```
