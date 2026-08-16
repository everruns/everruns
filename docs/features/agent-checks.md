---
title: Agent Checks
description: Advisory quality checks for agent configurations, structural problems, completeness gaps, and cost warnings surfaced while you build.
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
| `severity` | `warning`, `info`, or `suggestion`, there is no `error`; checks never block |
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

## Health Checks

A **health check** runs the agent for real. It generates a handful of smoke-test cases from the agent's description, system prompt, and capabilities, runs each as an actual session against the agent's configured model, and scores the result two ways:

- **Deterministic**: the agent produced a non-empty answer and finished within a turn budget.
- **AI judge**: the platform's utility LLM grades the agent's final response against a rubric generated for that case.

A case passes only when both checks pass. The run surfaces a score card (pass rate, passed count, average score, average turns) and a per-case list, each case links to the real session so you can inspect the full conversation, tool calls, and events.

Health checks are asynchronous (they run several real sessions and take a minute or two). Trigger one and poll for the result:

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/v1/agents/{agent_id}/health-checks` | Start a run; returns a pending run with an `id` |
| `GET` | `/v1/agents/{agent_id}/health-checks/{run_id}` | Poll the run; `status` goes `pending → running → completed`/`failed` |
| `GET` | `/v1/agents/{agent_id}/health-checks` | List recent runs for the agent |

Runs are stored per agent and keyed by the resolved config hash. Health checks require the utility LLM (`UTILITY_OPENAI_API_KEY`) to generate and judge cases, and the agent's own model must be usable. They are advisory: a low score never blocks anything.

## Built-in Rules

Checks run against the *resolved* configuration, after harness and capability contributions are merged, so they can catch issues that span layers.

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

High-cardinality rules (`prompt.duplicate_paragraphs`, `tools.unknown_reference`, `tools.duplicate_names`) cap how many findings they emit. When the cap is exceeded they add a single companion `info` finding with the rule ID suffixed `.summary` (e.g. `prompt.duplicate_paragraphs.summary`) noting that only the first N were shown, so a large prompt cannot amplify into an unbounded response.

## Roadmap

A later phase adds org-configurable rules: per-rule enable/severity settings for the built-ins plus custom declarative and natural-language-rubric rules.
