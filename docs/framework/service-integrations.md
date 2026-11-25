---
title: Application services
description: Compose GitHub, authenticated MCP, Slack, data services, native tools, triggers, traces, and evaluations without an Everruns server.
---

Framework applications can bind their own credentials, persistence, and retrieval services to the same operations the Server runs. The new APIs on this page are available from the repository’s `main` branch until the next crate release; run the linked examples from a source checkout or use the Git dependency shown in the [GitHub guide](/framework/github/). Enable only the Cargo features you use. Credentials stay in application-owned values and never enter an agent's serialized configuration.

| Priority | Feature | Framework API | Feature flag / guide |
| --- | --- | --- | --- |
| 1 | GitHub operations and rotating application credentials | `.capability(GitHub)` | `github`; [GitHub tools](/framework/github/) |
| 1 | MCP bearer credentials, OAuth login, refresh and rotation | `.mcp_auth(McpAuth)` | `mcp-auth`; [MCP authentication](/framework/mcp-authentication/) |
| 1 | Slack reactions, message edits, user lookup and external file uploads | `.slack_actions(SlackActions)` | `slack-actions`; [Slack](/framework/slack-services/) |
| 1 | Slack agent pane, streaming, status/title, approvals, progress, history and participation | `Channel::slack(Slack)` | `channels`; [Slack](/framework/slack-services/) |
| 2 | Event filters, changes-only checks, durable dedupe/outbox and session routing | `triggers::Dispatcher` | `triggers`; [Event triggers](/framework/event-triggers/) |
| 2 | Agents that start agents | `.spawns(Agent)`, `.threads_with(Agent)` | [Agents that start agents](/framework/coordination/) |
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

## Shared implementations and host boundaries

Server and Framework execute the shared operations below. Each host supplies its own identity, storage and lifecycle adapters; sharing an operation does not make the two hosts' persistence or permission models interchangeable.

| Priority | Feature | Shared implementation | Host-owned behavior |
| --- | --- | --- | --- |
| 1 | GitHub | `everruns-integrations::github`: the same schemas and complete vendor operations | Server resolves permitted connections; applications provide per-operation credentials and write opt-ins |
| 1 | MCP transport and OAuth | `everruns-core::mcp`: discovery, registration, PKCE, authorization parameters, callback validation, code exchange and refresh | Hosts select requested scopes and registration policy. Server owns browser callbacks, organization-scoped egress, encrypted records and replica locks; applications own callback delivery and token stores |
| 1 | Slack actions and delivery surfaces | `everruns-integrations::slack_channel`: API operations, uploads, Markdown, streams and agent-pane surfaces | Hosted routing, durable delivery recovery and authorization; application conversation bindings, participation and consent policies |
| 2 | Triggers | `everruns-core::triggers` and `triggers_github`: filters, changes-only comparisons, templates and GitHub normalization | Server persists hosted deliveries and routing; Framework's dispatcher owns its SQLite outbox and admission recovery |
| 2 | Named members | Core's delegate and session execution paths | Applications register named agents; Server resolves agent records and permissions |
| 2–3 | Cursor, Browserless, images and E2B | Existing native capabilities, vendor clients and tool operations | Credentials, network policy, sandbox ownership and cleanup |
| 3 | Session SQL | `everruns::database::runtime`: executor, authorizer, validation and limits; Server reexports these modules | Local directory storage versus hosted page/object storage, tenant checks and transactions |
| 3 | Memory | Core file-store and mount interfaces and the existing memory tools | Local named directories versus hosted storage, sharing and access policy |
| 3 | Knowledge | Existing Knowledge tools, retrieval contracts and citation types | Application lexical/custom retrievers versus managed ingestion, indexing and hosted retrieval; automatic retrieval persistence |
| 4 | Traces | `everruns-core::session_trace`: one event-to-turn/step projection | Framework validates a complete event prefix; Server maintains its persisted incremental projection |
| 4 | Evaluation | Core scoring predicates, file-content grading, citation evidence, judge prompts and output parsing | Evidence loading, unavailable-evidence policy, model selection, billing, result storage and worker scheduling; application-specific graders |

OAuth supports both public-client registration and confidential-client registration through the same protocol implementation. Discovery retains Server's protected-resource root fallback and issuer-path metadata fallbacks. Shared client errors retain recognized OAuth classifications without provider bodies, token values or transport URLs. Inbound Everruns `/oauth/*` authorization-server endpoints serve a different role from this outbound MCP client.

The [cross-host OAuth contract tests](https://github.com/everruns/everruns/blob/main/crates/server/src/api/user_connections/tests/shared_oauth_contract.rs) exercise embedded and browser login against the same endpoints. [Refresh integration tests](https://github.com/everruns/everruns/blob/main/crates/server/src/storage/connection_resolver/tests/shared_protocol.rs) verify that Server's two-replica lock still persists a rotation before releasing callers. These tests complement the Engine-level acceptance suites above.
