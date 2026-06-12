---
title: Agent Checks
description: Advisory quality checks for agent configurations — structural problems, completeness gaps, and cost warnings surfaced while you build.
---

# Agent Checks

Agent checks review an agent configuration and surface advisory findings while you build: structural problems (duplicated instructions, conflicting style guidance), completeness gaps (tool references that do not exist), and cost warnings (oversized prompts).

Checks are advisory only. Findings never block saving, publishing, or version creation.

## Where Findings Appear

- **Agent editor → Preview tab**: a Checks card lists findings for the current draft, updating as you edit.
- **API**: `POST /v1/agents/preview` returns a `findings` array alongside the resolved system prompt and tools.
- **MCP / platform commands**: the `preview_agent` command returns the same findings, so agents and automations can review configurations programmatically.

## Findings

Each finding includes:

| Field | Description |
|-------|-------------|
| `rule_id` | Stable rule identifier, e.g. `prompt.duplicate_paragraphs` |
| `severity` | `warning`, `info`, or `suggestion` — there is no `error`; checks never block |
| `category` | `structure`, `completeness`, `effectiveness`, `safety`, or `cost` |
| `message` | Human-readable explanation |
| `location` | The config field (and byte span, when applicable) the finding points at |

## Built-in Rules

Checks run against the *resolved* configuration — after harness and capability contributions are merged — so they can catch issues that span layers.

| Rule | Severity | What it catches |
|------|----------|-----------------|
| `prompt.empty` | info | Agent has no system prompt of its own |
| `prompt.very_long` | warning | Prompt over 32 KiB, sent on every model turn |
| `prompt.template_variables` | warning | `{{placeholder}}` text that would reach the model literally |
| `prompt.duplicate_paragraphs` | warning | The same paragraph appears more than once |
| `prompt.restates_contribution` | info | Prompt duplicates text already contributed by the harness or a capability |
| `prompt.conflicting_style` | info | Asks for both brevity and detail without stating conditions |
| `tools.unknown_reference` | info | Prompt references a tool that no enabled tool or capability provides |
| `tools.duplicate_names` | warning | Two tools share a name, so the model cannot distinguish them |

## Roadmap

Later phases add on-demand LLM-powered analysis (contradiction and structure review with proposed fixes), behavioral health checks that run the agent against generated smoke tests, and org-configurable rules.
