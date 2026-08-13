---
title: How-to guides
description: Task-oriented recipes for common Everruns workflows — equipping agents with tools, streaming events, deploying to channels, and operating production agents.
sidebar:
  order: 0
---

Each how-to here solves one concrete problem. They assume you already understand the basics (read the [Tutorials](/tutorials/run-an-agent/) first) and they don't try to teach concepts (see [Explanation](/explanation/) for that).

## Building agents

- [Equip an agent with tools](/how-to/equip-agents-with-tools/) — pick capabilities and assign them.
- [Give an agent web access](/how-to/give-an-agent-web-access/) — `web_fetch`, network policies, allowlists.
- [Define agents as files](/how-to/define-agents-as-files/) — version-controllable agent definitions in Markdown, TOML, or YAML.
- [Use AGENTS.md for project instructions](/how-to/use-agents-md/) — inject project-level context into the system prompt.
- [Customize a harness](/how-to/customize-a-harness/) — create your own harness as a starting point for many agents.
- [Share knowledge with OKF](/how-to/share-knowledge-with-okf/) — import/export Knowledge Bases as Open Knowledge Format bundles, managed like code.
- [Migrate between LLM providers](/how-to/migrate-providers/) — swap OpenAI ↔ Anthropic ↔ Gemini without rewriting agents.

## Running agents

- [Stream events with the SDK](/how-to/stream-events/) — consume the SSE stream from Python, with reconnection and event filtering.
- [Consume events via raw SSE](/how-to/consume-events-via-sse/) — when you don't want the SDK: curl, EventSource, or any HTTP client.
- [Handle errors and cancel turns](/how-to/handle-errors-and-cancellation/) — graceful failure paths, turn cancellation, retries.
- [Orchestrate multi-agent pipelines](/how-to/orchestrate-multi-agent-pipelines/) — chain sessions together.

## Packaging and distribution

- [Package an agent skill](/how-to/package-a-skill/) — author a SKILL.md, bundle scripts and references.
- [Publish a skill to the registry](/how-to/publish-a-skill-to-the-registry/) — share skills across agents.
- [Publish an agent as a Slack app](/how-to/publish-to-slack/) — deploy an agent to a Slack workspace.

## Upgrading

- [Migrate to 0.18](/how-to/migrate-to-0-18/) — move Rust code off the `everruns-core` paths that changed, with a symbol-by-symbol table of where each type now lives.

## Operating

- [Automate with the CLI](/how-to/automate-with-the-cli/) — scripting against the CLI with `jq`.
- [Deploy with Docker Compose](/getting-started/docker-compose/) — bring up the full platform.
- [Enforce a budget](/how-to/enforce-a-budget/) — cap token spend per agent, session, or organization.
