# Agents, sessions, and runtime resources

* [Agent Instructions Specification](agent-instructions.md) - AGENTS.md support (dynamic project instructions).
* [Agent Identities](agent-identities.md) - Agent identities (virtual principals for unattended execution).
* [Agent Blueprints](agent-blueprints.md) - Pre-built agent definitions.
* [Agent Versions](agent-versions.md) - Immutable Agent configuration snapshots.
* [Agent Handoff](agent-handoff.md) - Agent handoff behavior.
* [Agent Triggers](agent-triggers.md) - Agent triggers (agent wakes itself on a schedule; reuses the durable scheduler).
* [User Hooks Specification](user-hooks.md) - User-authored lifecycle hooks for agent execution.
* [Agent Reliability Tests](agent-reliability-tests.md) - Agent execution reliability tests.
* [Subagents Specification](subagents.md) - Subagent orchestration.
* [Session Tasks](session-tasks.md) - Session task registry for background work.
* [Session Participants](session-participants.md) - Session participants (host/member agents and users, addressed-turn routing, invite-mode handoff).
* [Session Resource Registry](session-resources.md) - Session resource registry.
* [Leased Resources](leased-resources.md) - Generic lease primitive.
* [Session Sandbox](session-sandbox.md) - Managed session-owned sandbox capability and lifecycle.
* [Container Sandbox Capability](container-sandbox.md) - Self-hosted Docker execution for agent sessions.
* [Workspace Specification](workspace.md) - Session workspace (file surface + tables).
* [Session Filesystem Specification](file-store.md) - Pluggable `SessionFileStore` backends.
* [Object Storage Specification](object-storage.md) - Optional S3-compatible blob backend for file/image content.
* [Session SQL Database Specification](session-sqldb.md) - Session-scoped SQL databases.
* [Session Export (JSONL)](session-export.md) - Session export to JSONL.
* [Forking Sessions](forking-sessions.md) - Fork a session into an independent copy (history + workspace + state).
* [Session Source and List Facets](session-source-and-facets.md) - How a session records the way it was started, and how the sessions list filters and counts over it.
* [Knowledge Bases Specification](knowledge-bases.md) - Curated organization knowledge.
* [Open Knowledge Format (OKF) Adoption](okf-adoption.md) - Open Knowledge Format (OKF) import/export adoption for Knowledge Bases.
* [Knowledge Indexes Specification](knowledge-indexes.md) - Source-backed, embedded, citable knowledge indexes.
* [Citations Specification](citations.md) - Claim-level source provenance as composable citation capabilities.
* [Memory Specification](memory.md) - Org-scoped named Memories (mountable into Workspaces).
* [Infinity Context](infinity-context.md) - Unlimited conversation length via context management.
* [Compaction](compaction.md) - Context compaction capability.
* [Client Hints](client-hints.md) - Generic client hints mechanism.
