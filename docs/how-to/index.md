---
title: How-to guides
description: Task-oriented recipes for common Everruns workflows, equipping agents with tools, streaming events, deploying to channels, and operating production agents.
sidebar:
  order: 0
---

These guides target the Everruns Platform (Everruns Cloud or self-hosted). Framework guides live under [/framework/](/framework/).

Each how-to here solves one concrete problem. They assume you already understand the basics (read the [Tutorials](/tutorials/run-an-agent/) first) and they don't try to teach concepts (see [Explanation](/explanation/) for that).

## Building agents

- [Equip an agent with tools](/how-to/equip-agents-with-tools/), pick capabilities and assign them.
- [Give an agent web access](/how-to/give-an-agent-web-access/), `web_fetch`, network policies, allowlists.
- [Define agents as files](/how-to/define-agents-as-files/), version-controllable agent definitions in Markdown, TOML, or YAML.
- [Use AGENTS.md for project instructions](/how-to/use-agents-md/), inject project-level context into the system prompt.
- [Customize a harness](/how-to/customize-a-harness/), create your own harness as a starting point for many agents.
- [Share knowledge with OKF](/how-to/share-knowledge-with-okf/), import/export Knowledge Bases as Open Knowledge Format bundles, managed like code.
- [Migrate between LLM providers](/how-to/migrate-providers/), swap OpenAI ↔ Anthropic ↔ Gemini without rewriting agents.

## Running agents

- [Call your agent from code](/how-to/call-your-agent-from-code/), give an application its own key to one agent, act for your users, and reach it from a browser.
- [Stream events](/how-to/stream-events/), consume the SSE stream from the Python SDK, or from curl, EventSource, or any HTTP client, with reconnection and event filtering.
- [Complete a URL elicitation over the API](/how-to/complete-a-url-elicitation/), drive the pause-and-consent flow from your own client.
- [Handle errors and cancel turns](/how-to/handle-errors-and-cancellation/), graceful failure paths, turn cancellation, retries.
- [Orchestrate multi-agent pipelines](/how-to/orchestrate-multi-agent-pipelines/), chain sessions together.
- [Build a foreman agent](/how-to/build-a-foreman-agent/), put one agent in front of a team of specialists and let it triage and delegate.

## Packaging and distribution

- [Package and publish an agent skill](/how-to/package-a-skill/), author a SKILL.md, bundle scripts and references, and share it across agents through the registry.
- [Publish an agent as a Slack app](/how-to/publish-to-slack/), deploy an agent to a Slack workspace.
- [Summarize GitHub pull requests](/how-to/summarize-github-pull-requests/), connect GitHub and comment a summary on every pull request.
- [Set up review and security agents](/how-to/set-up-review-and-security-agents/), review every pull request and scan a repository for vulnerabilities on a schedule, on any model.

## Operating

- [Automate with the CLI](/how-to/automate-with-the-cli/), scripting against the CLI with `jq`.
- [Deploy with Docker Compose](/getting-started/docker-compose/), bring up the full platform.
- [Enforce a budget](/how-to/enforce-a-budget/), cap token spend per agent, session, or organization.
