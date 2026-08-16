---
title: Define agents as files
description: Author agent definitions in Markdown, TOML, YAML, or JSON so they can be version-controlled, reviewed in pull requests, and imported via the SDK or CLI.
---

Agents can be defined as files with structured metadata and a system prompt. This makes them shareable, reviewable, and version-controllable, useful for teams that want agents in git rather than only in the API.

## Markdown with front matter

The most readable format. The YAML front matter holds metadata; the body becomes the system prompt.

```markdown
---
name: "hackernews-reader"
description: "An agent that browses HackerNews autonomously"
tags:
  - demo
  - hackernews
capabilities:
  - web_fetch
  - current_time
  - session_file_system
---
You are a HackerNews reader agent. You autonomously browse
Hacker News to find interesting stories, read discussions,
and research authors.
```

Import via the SDK:

```python
with open("hackernews-reader.md") as f:
    agent = await client.agents.import_agent(f.read())
```

Or via the CLI:

```bash
everruns agents create -f hackernews-reader.md
```

## TOML

```toml
name = "research-assistant"
description = "Helps with research tasks"
# Base execution harness for this agent (id or name). Omit to default to the
# org's built-in `generic` harness. Sessions started from the agent inherit it.
harness_name = "generic"
system_prompt = """
You are a helpful research assistant.
Always cite your sources.
"""
tags = ["research", "assistant"]

[[capabilities]]
ref = "current_time"

[[capabilities]]
ref = "web_fetch"
```

If `./agent.toml` exists in the current directory and you don't pass inline flags, `everruns agents create` picks it up automatically.

## YAML

```yaml
name: "research-assistant"
description: "Helps with research tasks"
# Base execution harness (id or name); omit to default to the built-in `generic`.
harness_name: "generic"
system_prompt: |
  You are a helpful research assistant.
  Always cite your sources.
capabilities:
  - ref: current_time
    config: {}
  - ref: web_fetch
    config: {}
tags:
  - research
```

Shorthand form (capability IDs only):

```yaml
capabilities:
  - current_time
  - web_fetch
```

The long form (`ref` + `config`) is required for per-agent capability configuration.

## JSON

JSON is supported for tooling that generates agent definitions programmatically. It has no special features over TOML/YAML, pick the format your team prefers.

```bash
everruns agents create -f agent.json
```

## Seed sessions with initial files

To pre-populate the session workspace, either pass `--initial-files-dir` on the CLI or use the `initial_files` front matter field:

```markdown
---
name: "a11y-audit"
capabilities:
  - daytona
initial_files:
  - .
  - .agents/*
---
Run axe-core audits...
```

Entries can be:

- `.`, the entire current directory (non-hidden files plus `.agents/`).
- A subdirectory, walked recursively. Glob suffixes like `/*` are stripped.
- A single file path.

Hidden files outside `.agents/` are skipped; symlinks outside the base directory are rejected; binary files are ignored.

## Update vs. create

`everruns agents update` accepts the same file formats. Passing an explicit `<id>` positional disables implicit `agent.toml` selection so you can update a different agent from the same directory.

## See also

- [CLI reference](/features/cli/), full command surface.
- [Equip an agent with tools](/how-to/equip-agents-with-tools/), choosing capabilities.
- [Use AGENTS.md for project instructions](/how-to/use-agents-md/), adding per-project guidance.
