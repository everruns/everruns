---
title: everruns harnesses
description: "Reusable base setups (prompt plus capabilities) agents build on. CLI reference for everruns harnesses: usage, flags and examples."
sidebar:
  label: harnesses
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Reusable base setups (prompt plus capabilities) agents build on.

| Command | What it does |
|---|---|
| [`harnesses copy`](#harnesses-copy) | Copy a harness. |
| [`harnesses create`](#harnesses-create) | Create a new harness with a name, system prompt, and optional capabilities. |
| [`harnesses delete`](#harnesses-delete) | Archive a harness (soft delete). |
| [`harnesses destroy`](#harnesses-destroy) | Permanently delete an archived harness. |
| [`harnesses get`](#harnesses-get) | Get a single harness by ID or name. |
| [`harnesses list`](#harnesses-list) | List all active harnesses in the org. |
| [`harnesses preview`](#harnesses-preview) | Preview the final harness shape with capabilities applied. |
| [`harnesses update`](#harnesses-update) | Update a harness. |
| [`harnesses check-name check`](#harnesses-check-name-check) | Check whether a harness name is available. |
| [`harnesses examples adopt`](#harnesses-examples-adopt) | Create an org harness from a harness example by name (see list_harness_examples), e.g. |
| [`harnesses examples list`](#harnesses-examples-list) | List adoptable harness examples (ready-made harnesses such as coding, the Yolop-based coding harness, and data-analyst): name, description, parent harness, capabilities. |

## harnesses copy

Copy a harness. Generates a unique name.

```bash
everruns harnesses copy [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Start a new harness from an existing one
everruns harnesses copy harness_01h9 --reason 'Fork for the EU deployment'
```

## harnesses create

Create a new harness with a name, system prompt, and optional capabilities.

```bash
everruns harnesses create [OPTIONS] --name <name>
```

| Flag | Description |
|---|---|
| `--capabilities <CAPABILITIES>` | Capabilities to enable with per-harness configuration. |
| `--default-model-id <DEFAULT_MODEL_ID>` | Default LLM model ID for this harness. |
| `--description <DESCRIPTION>` | Description of what the harness does. |
| `--display-name <DISPLAY_NAME>` | Human-readable display name shown in UI. |
| `--embedder-metadata <EMBEDDER_METADATA>` | Arbitrary key-value metadata injected into LLM requests for observability. |
| `--initial-files <INITIAL_FILES>` | Starter files copied into each new session for this harness. |
| `--intro-markdown <INTRO_MARKDOWN>` | Deprecated: configure conversation presentation on the Agent instead. |
| `--localizations <LOCALIZATIONS>` | Locale-keyed display overlays. |
| `--mcpServers <MCPSERVERS>` |  |
| `--name <NAME>` | Required. Name, unique per org. |
| `--network-access <NETWORK_ACCESS>` |  |
| `--parent-harness-id <PARENT_HARNESS_ID>` | Optional parent harness to inherit from. |
| `--short-description <SHORT_DESCRIPTION>` | Deprecated: configure conversation presentation on the Agent instead. |
| `--starters <STARTERS>` | Deprecated: configure conversation presentation on the Agent instead. |
| `--system-prompt <SYSTEM_PROMPT>` | Base system prompt defining the harness's behavior. |
| `--tags <TAGS>` | Tags for organizing harnesses. Comma-separated or a JSON array of strings. Repeatable. |

Example:

```bash
# Create a harness with a prompt and a capability
everruns harnesses create --name support-bot --system-prompt 'You answer support questions' --capabilities '[{"ref":"web_fetch"}]' --reason 'Base harness for support agents'
```

## harnesses delete

Archive a harness (soft delete). Can be restored.

```bash
everruns harnesses delete [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Archive a harness you no longer use; it can be restored
everruns harnesses delete harness_01h9 --reason 'Replaced by support-bot'
```

## harnesses destroy

Permanently delete an archived harness.

```bash
everruns harnesses destroy [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Permanently remove an already-archived harness
everruns harnesses destroy harness_01h9 --reason 'Retired after the archive window'
```

## harnesses get

Get a single harness by ID or name.

```bash
everruns harnesses get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Show a harness by id or name
everruns harnesses get harness_01h9
```

## harnesses list

List all active harnesses in the org. Use search for name search, include_archived=true to include archived. Ready-made harnesses such as coding are examples and appear here only after adoption: see list_harness_examples.

```bash
everruns harnesses list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-archived` | Include archived harnesses. |
| `--search <SEARCH>` | Only harnesses whose name matches this text. |

Example:

```bash
# Find harnesses by name when you do not know the id
everruns harnesses list --search support
```

## harnesses preview

Preview the final harness shape with capabilities applied.

```bash
everruns harnesses preview [OPTIONS]
```

| Flag | Description |
|---|---|
| `--capabilities <CAPABILITIES>` | Capabilities to apply, each `{"ref": ..., "config": ...}`. |
| `--mcp-servers <MCP_SERVERS>` |  |
| `--parent-harness-id <PARENT_HARNESS_ID>` |  |
| `--system-prompt <SYSTEM_PROMPT>` | Base system prompt to preview. |

Example:

```bash
# See the final prompt and tools a harness would produce before saving it
everruns harnesses preview --system-prompt 'You answer support questions' --capabilities '[{"ref":"web_fetch"}]'
```

## harnesses update

Update a harness. Only provided fields are changed.

```bash
everruns harnesses update [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |
| `--capabilities <CAPABILITIES>` | Replace the capability list entirely; omit to leave unchanged. |
| `--default-model-id <DEFAULT_MODEL_ID>` | New default model selected when sessions inherit from this harness; omit to leave unchanged. |
| `--description <DESCRIPTION>` | Human-readable description. |
| `--display-name <DISPLAY_NAME>` | Human-readable display name. |
| `--embedder-metadata <EMBEDDER_METADATA>` | Replace the embedder metadata map entirely; omit to leave unchanged. |
| `--initial-files <INITIAL_FILES>` | Replace the initial-files list entirely; omit to leave unchanged. |
| `--intro-markdown <INTRO_MARKDOWN>` | Deprecated: configure conversation presentation on the Agent instead. |
| `--localizations <LOCALIZATIONS>` |  |
| `--mcpServers <MCPSERVERS>` |  |
| `--name <NAME>` | Name, unique per org. |
| `--network-access <NETWORK_ACCESS>` |  |
| `--parent-harness-id <PARENT_HARNESS_ID>` | New parent harness for inheritance. |
| `--short-description <SHORT_DESCRIPTION>` | Deprecated: configure conversation presentation on the Agent instead. |
| `--starters <STARTERS>` | Deprecated: configure conversation presentation on the Agent instead. |
| `--status <STATUS>` |  |
| `--system-prompt <SYSTEM_PROMPT>` | New system prompt the harness contributes to sessions; omit to leave unchanged. |
| `--tags <TAGS>` | Replace the tag list entirely; omit to leave unchanged. Comma-separated or a JSON array of strings. Repeatable. |

Example:

```bash
# Change a harness's system prompt
everruns harnesses update harness_01h9 --system-prompt 'You answer support questions concisely' --reason 'Tighten replies'
```

## harnesses check-name check

Check whether a harness name is available.

```bash
everruns harnesses check-name check [OPTIONS] --name <name>
```

| Flag | Description |
|---|---|
| `--exclude-id <EXCLUDE_ID>` | Harness id to ignore, so a harness can keep its own name. |
| `--name <NAME>` | Required. Human-readable name. |

Example:

```bash
# Check a name is free before creating a harness
everruns harnesses check-name check --name support-bot
```

## harnesses examples adopt

Create an org harness from a harness example by name (see list_harness_examples), e.g. coding for the Yolop-based coding harness. The harness is named after the example, with a short suffix when that name is taken, and inherits from the example's parent harness. Agents then select it by that name.

```bash
everruns harnesses examples adopt [OPTIONS] [NAME]
```

| Flag | Description |
|---|---|
| `--name <NAME>` | Example name from list_harness_examples, such as `coding`. |

Example:

```bash
# Add the Coding harness to the org so agents can use it
everruns harnesses examples adopt coding --reason 'Coding agents need the Coding harness'
```

## harnesses examples list

List adoptable harness examples (ready-made harnesses such as coding, the Yolop-based coding harness, and data-analyst): name, description, parent harness, capabilities. They are not in list_harnesses until adopted; adopt one by name with adopt_harness_example before building a harness from scratch.

```bash
everruns harnesses examples list [OPTIONS]
```

Example:

```bash
# Find a ready-made harness (such as coding) before writing one by hand
everruns harnesses examples list
```
