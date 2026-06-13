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
- **MCP / platform commands**: the `preview_agent` and `analyze_agent` commands return the same findings, so agents and automations can review configurations programmatically.

## Findings

Each finding includes:

| Field | Description |
|-------|-------------|
| `rule_id` | Stable rule identifier, e.g. `prompt.duplicate_paragraphs` |
| `severity` | `warning`, `info`, or `suggestion` — there is no `error`; checks never block |
| `category` | `structure`, `completeness`, `effectiveness`, `safety`, or `cost` |
| `message` | Human-readable explanation |
| `location` | The config field (and byte span, when applicable) the finding points at |

## AI Analysis

The **Analyze** button on the Checks card runs a deeper on-demand review using the platform's internal utility LLM (requires `UTILITY_OPENAI_API_KEY` on the deployment). Three scoped checkers run in parallel:

| Rule | What it catches |
|------|-----------------|
| `llm.contradiction` | Instructions that cannot both be followed, including conflicts between the prompt and capability contributions |
| `llm.structure` | Redundancy, verbosity, vague instructions, and structure that buries critical rules |
| `llm.tool_guidance` | Prompt guidance that misdescribes available tools or assumes functionality no tool provides |

LLM findings can carry a suggested replacement for the offending text; when the finding is anchored to a span of your prompt, an **Apply fix** button replaces it in place. Analysis is available via `POST /v1/agents/analyze`, which returns built-in and LLM findings merged.

The reviewed prompt is treated strictly as data: checker outputs are bounded, severities are clamped, and findings are advisory text only.

## Built-in Rules

Checks run against the *resolved* configuration — after harness and capability contributions are merged — so they can catch issues that span layers.

| Rule | Severity | What it catches |
|------|----------|-----------------|
| `prompt.empty` | info | Agent has no system prompt of its own |
| `prompt.very_long` | warning | Authored prompt over 32 KiB, sent on every model turn |
| `prompt.resolved_very_long` | info | Full prompt over 96 KiB after harness/capability contributions |
| `prompt.template_variables` | warning | `{{placeholder}}` text that would reach the model literally |
| `prompt.duplicate_paragraphs` | warning | The same paragraph appears more than once |
| `prompt.restates_contribution` | info | Prompt duplicates text already contributed by the harness or a capability |
| `prompt.conflicting_style` | info | Asks for both brevity and detail without stating conditions |
| `tools.unknown_reference` | info | Prompt references a tool that no enabled tool or capability provides |
| `tools.duplicate_names` | warning | Two tools share a name, so the model cannot distinguish them |

## Roadmap

Later phases add behavioral health checks that run the agent against generated smoke tests, and org-configurable rules.
