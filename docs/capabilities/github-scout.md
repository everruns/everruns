---
title: GitHub Scout
description: Blueprint-only GitHub repository exploration capability that spawns read-only scout subagents.
---

| | |
|---|---|
| **ID** | `github_scout` |
| **Category** | Integrations |
| **Features** | None |
| **Dependencies** | [`subagents`](/capabilities/sub-agents/) |

GitHub Scout lets an agent spawn a specialist subagent for read-only GitHub repository exploration. The host agent receives `spawn_agent` with `target.type: "subagent"` through the dependency on `subagents`; monitoring and steering are handled by the generic `session_tasks` tools (`list_tasks`, `get_task`, `message_task`, `cancel_task`). The GitHub API tools stay private inside the spawned `github_scout` blueprint session.

## How to Use

Enable the `github_scout` capability on an agent or harness. Then spawn the blueprint:

```json
{
  "name": "spawn_agent",
  "arguments": {
    "name": "Scout",
    "instructions": "Find where authentication middleware is implemented.",
    "target": { "type": "subagent" },
    "blueprint": "github_scout",
    "config": {
      "repos": ["fastify/fastify"]
    }
  }
}
```

The optional `repos` config scopes GitHub searches to `owner/repo` repositories.

## Private Blueprint Tools

These tools are available only inside the GitHub Scout child session:

| Tool | Description |
|---|---|
| `search_github_code` | Search code with GitHub code search qualifiers such as `repo:`, `path:`, `language:`, `symbol:`, and `filename:` |
| `read_github_file` | Read a UTF-8 file from a repository by `repo`, `path`, and optional `ref` |
| `search_github_issues` | Search issues and pull requests with qualifiers such as `repo:`, `is:issue`, `is:pr`, `state:`, `author:`, and `label:` |

## Authentication

GitHub Scout uses the existing GitHub user connection. In local and compatibility flows, tools can also use a `GITHUB_TOKEN` session secret. Missing credentials return a connection prompt instead of asking for tokens in chat.

## Included Examples

The adoptable **Coding (Daytona)** and **Coding (Container)** harness examples include `github_scout`, so coding agents based on those examples can delegate GitHub repository lookup work to Scout.

## See Also

- [Sub Agents](/capabilities/sub-agents/), lifecycle tools used to spawn and manage Scout
- [Capabilities Overview](/capabilities/), full capability catalog
