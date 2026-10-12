---
title: everruns capabilities
description: "Capabilities agents can use, including declarative ones you define. CLI reference for everruns capabilities."
sidebar:
  label: capabilities
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Capabilities agents can use, including declarative ones you define.

| Command | What it does |
|---|---|
| [`capabilities create`](#capabilities-create) | Create a persisted declarative capability. |
| [`capabilities get`](#capabilities-get) | Get a specific capability by ID. |
| [`capabilities list`](#capabilities-list) | List available capabilities. |
| [`capabilities declarative delete`](#capabilities-declarative-delete) | Archive a persisted declarative capability. |
| [`capabilities declarative destroy`](#capabilities-declarative-destroy) | Permanently delete an archived declarative capability. |
| [`capabilities declarative get`](#capabilities-declarative-get) | Get a persisted declarative capability. |
| [`capabilities declarative list`](#capabilities-declarative-list) | List persisted declarative capabilities. |
| [`capabilities declarative update`](#capabilities-declarative-update) | Update a persisted declarative capability. |
| [`capabilities guardrails dry-run dry`](#capabilities-guardrails-dry-run-dry) | Evaluate a guardrails capability config against sample text without a session. |
| [`capabilities guardrails examples list`](#capabilities-guardrails-examples-list) | List adoptable guardrail presets (the guardrail gallery), each with its config and trust metadata (check-type composition, stages, data egress). |

## capabilities create

Create a persisted declarative capability.

```bash
everruns capabilities create [OPTIONS] --definition <definition>
```

| Flag | Description |
|---|---|
| `--definition <DEFINITION>` | Required. Definition for the new declarative capability. |

Example:

```bash
# Package instructions and starter files as a reusable capability
everruns capabilities create --definition '{"name":"research_pack","display_name":"Research Pack","description":"Research workflow","system_prompt":"Use the research workflow."}' --reason 'Share the research workflow'
```

## capabilities get

Get a specific capability by ID.

```bash
everruns capabilities get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Read what a capability provides before adding it to an agent
everruns capabilities get cap_01h9
```

## capabilities list

List available capabilities. Use search for name/description filtering. Retired capabilities are excluded unless include_retired is set. Supports pagination (limit/offset).

```bash
everruns capabilities list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-retired` | Include capabilities that have been retired (removed but still referenceable). |
| `--limit <LIMIT>` | Maximum number of items returned in this page. |
| `--offset <OFFSET>` | Zero-based offset into the result set. |
| `--search <SEARCH>` | Case-insensitive substring match on name or description. |

Example:

```bash
# Find a capability by name when you do not know its id
everruns capabilities list --search web
```

## capabilities declarative delete

Archive a persisted declarative capability.

```bash
everruns capabilities declarative delete [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Archive a declarative capability that agents should stop using
everruns capabilities declarative delete cap_01h9 --reason 'Replaced by research_pack v2'
```

## capabilities declarative destroy

Permanently delete an archived declarative capability.

```bash
everruns capabilities declarative destroy [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Permanently remove an archived declarative capability
everruns capabilities declarative destroy cap_01h9 --reason 'Retired after the archive window'
```

## capabilities declarative get

Get a persisted declarative capability.

```bash
everruns capabilities declarative get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Read the saved definition of a declarative capability
everruns capabilities declarative get cap_01h9
```

## capabilities declarative list

List persisted declarative capabilities.

```bash
everruns capabilities declarative list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-archived` | Also return archived items. |
| `--search <SEARCH>` | Case-insensitive substring match on name or description. |

Example:

```bash
# List the declarative capabilities your organization has defined
everruns capabilities declarative list --search research
```

## capabilities declarative update

Update a persisted declarative capability.

```bash
everruns capabilities declarative update [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |
| `--definition <DEFINITION>` | Replacement declarative definition. |
| `--status <STATUS>` | Optional lifecycle state update. |

Example:

```bash
# Replace a declarative capability's definition
everruns capabilities declarative update cap_01h9 --definition '{"name":"research_pack","display_name":"Research Pack","description":"Research workflow","system_prompt":"Use the updated research workflow."}' --reason 'Tighten the instructions'
```

## capabilities guardrails dry-run dry

Evaluate a guardrails capability config against sample text without a session. Returns triggered checks and whether the content would be blocked.

```bash
everruns capabilities guardrails dry-run dry [OPTIONS] --config <config> --stage <stage> --text <text>
```

| Flag | Description |
|---|---|
| `--config <CONFIG>` | Required. The `guardrails` capability config to evaluate (same shape persisted in `AgentCapabilityConfi... |
| `--stage <STAGE>` | Required. Pipeline stage a check applies to. One of `output`, `tool_use`, `tool_output`. |
| `--text <TEXT>` | Required. Sample content: model output, serialized tool arguments, or tool output, depending on `stage`. |
| `--tool-name <TOOL_NAME>` | Tool name for `tool_use` stage checks (`tool_pattern` rules). |

Example:

```bash
# Try a guardrails config on sample text before attaching it to an agent
everruns capabilities guardrails dry-run dry --stage output --text 'this is darn slow' --config '{"mode":"advisory","checks":[{"id":"profanity","stage":"output","type":"blocklist","words":["darn"]}]}'
```

## capabilities guardrails examples list

List adoptable guardrail presets (the guardrail gallery), each with its config and trust metadata (check-type composition, stages, data egress).

```bash
everruns capabilities guardrails examples list [OPTIONS]
```

Example:

```bash
# Start from a ready-made guardrail preset instead of writing checks by hand
everruns capabilities guardrails examples list
```
