---
title: everruns agents
description: "Agent definitions and their configuration. CLI reference for everruns agents."
sidebar:
  label: agents
appliesTo: [platform, cloud]
---

<!-- Generated from the everruns CLI by `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`. Edit command descriptions and examples in the code, not here. -->

Agent definitions and their configuration.

| Command | What it does |
|---|---|
| [`agents import`](#agents-import) | Import a portable agent file, folder or ZIP. |
| [`agents export`](#agents-export) | Export an agent by name without IDs or credentials. |
| [`agents validate`](#agents-validate) | Validate a package locally, or also check destination dependencies. |
| [`agents diff`](#agents-diff) | Generate a semantic diff against a local package or a remote agent. |
| [`agents create`](#agents-create) | Create a new agent (upserts if id: is present in frontmatter) |
| [`agents update`](#agents-update) | Update an existing agent from a file definition. |
| [`agents analyze`](#agents-analyze) | Run advisory checks (built-in rules plus LLM analysis) against an agent configuration. |
| [`agents check-name`](#agents-check-name) | Check whether an agent name is available. |
| [`agents copy`](#agents-copy) | Copy an agent. |
| [`agents delete`](#agents-delete) | Archive an agent (soft delete). |
| [`agents destroy`](#agents-destroy) | Permanently delete an archived agent. |
| [`agents get`](#agents-get) | Get a single agent by ID or name. |
| [`agents list`](#agents-list) | List all active agents. |
| [`agents preview`](#agents-preview) | Preview the final agent shape with capabilities applied. |
| [`agents upsert`](#agents-upsert) | Upsert agent — create (201) or update (200) by ID. |
| [`agents channels create`](#agents-channels-create) | Create an ingress channel for an agent. |
| [`agents channels delete`](#agents-channels-delete) | Delete an agent ingress channel. |
| [`agents channels get`](#agents-channels-get) | Get an agent ingress channel. |
| [`agents channels list`](#agents-channels-list) | List an agent's ingress channels. |
| [`agents channels publish`](#agents-channels-publish) | Publish an agent ingress channel. |
| [`agents channels trigger`](#agents-channels-trigger) | Run an agent schedule channel now. |
| [`agents channels unpublish`](#agents-channels-unpublish) | Unpublish an agent ingress channel. |
| [`agents channels update`](#agents-channels-update) | Update an agent ingress channel. |
| [`agents check-rules delete`](#agents-check-rules-delete) | Delete an agent check rule (built-in override or custom rule). |
| [`agents check-rules list`](#agents-check-rules-list) | List the org's agent check rules: built-in rules with their effective enabled/severity, plus custom rules. |
| [`agents check-rules upsert`](#agents-check-rules-upsert) | Create or update an agent check rule (built-in override or custom rule). |
| [`agents credentials create`](#agents-credentials-create) | Create a write-only Agent credential setup requirement for one attached MCP tool parameter. |
| [`agents exposures resume`](#agents-exposures-resume) | Let an agent's live channels accept traffic again. |
| [`agents exposures suspend`](#agents-exposures-suspend) | Stop every channel on an agent from accepting traffic. |
| [`agents health-checks get`](#agents-health-checks-get) | Get a single agent health check run with its results. |
| [`agents health-checks list`](#agents-health-checks-list) | List recent health check runs for an agent. |
| [`agents health-checks trigger`](#agents-health-checks-trigger) | Run a behavioral health check (generated smoke tests) against an agent. |
| [`agents health-checks latest get`](#agents-health-checks-latest-get) | Get the latest health check run for an agent, with a stale-config flag. |
| [`agents scripts create`](#agents-scripts-create) | Create a saved shell script an agent owns. |
| [`agents scripts delete`](#agents-scripts-delete) | Archive a saved script and free its name. |
| [`agents scripts get`](#agents-scripts-get) | Get a single saved script by id. |
| [`agents scripts list`](#agents-scripts-list) | List an agent's saved scripts with their bodies. |
| [`agents scripts update`](#agents-scripts-update) | Update a saved script's description, input schema or body. |
| [`agents triggers create`](#agents-triggers-create) | Create a schedule or webhook trigger for an agent. |
| [`agents triggers delete`](#agents-triggers-delete) | Archive an agent trigger and remove its durable schedule. |
| [`agents triggers get`](#agents-triggers-get) | Get a single agent trigger by id. |
| [`agents triggers list`](#agents-triggers-list) | List an agent's triggers. |
| [`agents triggers trigger`](#agents-triggers-trigger) | Manually fire an agent trigger once. |
| [`agents triggers update`](#agents-triggers-update) | Update an agent trigger. |
| [`agents triggers deliveries list`](#agents-triggers-deliveries-list) | List recent events delivered to an agent trigger: dispatched, filtered, duplicate or failed. |
| [`agents triggers runs list`](#agents-triggers-runs-list) | List recent execution outcomes for an agent trigger. |

## agents import

Import a portable agent file, folder or ZIP.

```bash
everruns agents import [OPTIONS] [PATH]
```

| Flag | Description |
|---|---|
| `<PATH>` |  |
| `--content <CONTENT>` | Inline definition (also supported by Platform Chat and MCP) |
| `--target <TARGET>` | Explicit existing agent name to update. |
| `--format <FORMAT>` |  |


## agents export

Export an agent by name without IDs or credentials.

```bash
everruns agents export [OPTIONS] <AGENT>
```

| Flag | Description |
|---|---|
| `<AGENT>` | Required. |
| `--format <FORMAT>` | One of `markdown`, `toml`, `yaml`, `json`, `zip`, `folder`. |
| `--out <OUT>` |  |


## agents validate

Validate a package locally, or also check destination dependencies.

```bash
everruns agents validate [OPTIONS] <FILE>
```

| Flag | Description |
|---|---|
| `<FILE>` | Required. |
| `--remote` |  |


## agents diff

Generate a semantic diff against a local package or a remote agent.

```bash
everruns agents diff [OPTIONS] <FILE>
```

| Flag | Description |
|---|---|
| `<FILE>` | Required. |
| `--against <AGAINST>` |  |
| `--target <TARGET>` |  |


## agents create

Create a new agent (upserts if id: is present in frontmatter)

```bash
everruns agents create [OPTIONS]
```

| Flag | Description |
|---|---|
| `-f`, `--file <FILE>` | TOML/YAML/JSON/Markdown file with agent definition. |
| `--initial-files-dir <INITIAL_FILES_DIR>` | Directory of files to upload as initial_files (read-only by default) |
| `--writable` | Make initial files writable (default: read-only) |
| `--name <NAME>` | Agent name (required if no --file) |
| `--instructions <SYSTEM_PROMPT>` | System prompt (required if no --file) |
| `--description <DESCRIPTION>` | Agent description. |
| `--model <MODEL>` | Default model ID (e.g. mod_xxx) |
| `-H`, `--harness <HARNESS>` | Harness ID or name (e.g. harness_xxx or "generic"). Omit to default to the org's generic harness. |
| `-t`, `--tag <TAG>` | Tags (repeatable) Repeatable. |


## agents update

Update an existing agent from a file definition.

```bash
everruns agents update [OPTIONS] [AGENT_ID]
```

| Flag | Description |
|---|---|
| `<AGENT_ID>` | Agent ID (e.g. agent_xxx). If omitted, uses id from file frontmatter. |
| `-f`, `--file <FILE>` | TOML/YAML/JSON/Markdown file with agent definition. |
| `--initial-files-dir <INITIAL_FILES_DIR>` | Directory of files to upload as initial_files (read-only by default) |
| `--writable` | Make initial files writable (default: read-only) |
| `--name <NAME>` | Agent name. |
| `--instructions <SYSTEM_PROMPT>` | System prompt. |
| `--description <DESCRIPTION>` | Agent description. |
| `--model <MODEL>` | Default model ID (e.g. mod_xxx) |
| `-H`, `--harness <HARNESS>` | Harness ID or name (e.g. harness_xxx or "generic") |
| `-t`, `--tag <TAG>` | Tags (repeatable) Repeatable. |


## agents analyze

Run advisory checks (built-in rules plus LLM analysis) against an agent configuration.

```bash
everruns agents analyze [OPTIONS]
```

| Flag | Description |
|---|---|
| `--capabilities <CAPABILITIES>` |  |
| `--harness-id <HARNESS_ID>` |  |
| `--initial-files <INITIAL_FILES>` |  |
| `--mcp-servers <MCP_SERVERS>` |  |
| `--system-prompt <SYSTEM_PROMPT>` |  |
| `--tools <TOOLS>` |  |

Example:

```bash
# Check a draft configuration for problems before creating the agent
everruns agents analyze --system-prompt 'Triage incoming issues' --tools '["bash"]'
```

## agents check-name

Check whether an agent name is available.

```bash
everruns agents check-name [OPTIONS] --name <name>
```

| Flag | Description |
|---|---|
| `--exclude-id <EXCLUDE_ID>` |  |
| `--name <NAME>` | Required. Human-readable name. |

Example:

```bash
# See whether a name is free before creating an agent
everruns agents check-name --name triage
```

## agents copy

Copy an agent. Generates a unique name.

```bash
everruns agents copy [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Duplicate an agent to try a change without touching the original
everruns agents copy agent_01h9 --reason 'Trial a stricter prompt'
```

## agents delete

Archive an agent (soft delete). Can be restored.

```bash
everruns agents delete [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Archive an agent, keeping it restorable
everruns agents delete agent_01h9 --reason 'Replaced by triage-v2'
```

## agents destroy

Permanently delete an archived agent.

```bash
everruns agents destroy [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Permanently remove an already-archived agent
everruns agents destroy agent_01h9 --reason 'Retired after the archive window'
```

## agents get

Get a single agent by ID or name.

```bash
everruns agents get [OPTIONS] [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Prefixed public identifier. |

Example:

```bash
# Show one agent's full configuration
everruns agents get agent_01h9
```

## agents list

List all active agents. Use search for name search, include_archived=true to include archived. Supports pagination (limit/offset).

```bash
everruns agents list [OPTIONS]
```

| Flag | Description |
|---|---|
| `--include-archived` |  |
| `--limit <LIMIT>` | Maximum number of items returned in this page. |
| `--offset <OFFSET>` | Zero-based offset into the result set. |
| `--search <SEARCH>` |  |

Example:

```bash
# Find agents by name when you do not know the id
everruns agents list --search triage --limit 20
```

## agents preview

Preview the final agent shape with capabilities applied.

```bash
everruns agents preview [OPTIONS]
```

| Flag | Description |
|---|---|
| `--capabilities <CAPABILITIES>` |  |
| `--harness-id <HARNESS_ID>` |  |
| `--initial-files <INITIAL_FILES>` |  |
| `--mcp-servers <MCP_SERVERS>` |  |
| `--system-prompt <SYSTEM_PROMPT>` |  |
| `--tools <TOOLS>` |  |

Example:

```bash
# See the prompt a draft configuration would produce, without creating it
everruns agents preview --system-prompt 'Triage incoming issues'
```

## agents upsert

Upsert agent — create (201) or update (200) by ID.

```bash
everruns agents upsert [OPTIONS] --name <name> --system-prompt <system_prompt> [ID]
```

| Flag | Description |
|---|---|
| `--id <ID>` | Client-supplied agent ID (format: agent_{32-hex}). If not provided, one is auto-generated. |
| `--capabilities <CAPABILITIES>` | Capabilities to enable for this agent with per-agent configuration. Each capability has a `re... |
| `--default-model-id <DEFAULT_MODEL_ID>` | The ID of the default LLM model to use for this agent. If not specified, the system default m... |
| `--description <DESCRIPTION>` | A human-readable description of what the agent does. |
| `--display-name <DISPLAY_NAME>` | Human-readable display name shown in UI. Falls back to `name` when absent. |
| `-H`, `--harness <HARNESS_NAME>` | Addressable harness name. |
| `--harness-id <HARNESS_ID>` | Harness ID used as this agent's base execution environment. |
| `--initial-files <INITIAL_FILES>` | Starter files copied into each new session for this agent. |
| `--intro-markdown <INTRO_MARKDOWN>` | Markdown intro shown as an intro box on a fresh Platform Chat thread. Images are allowed. |
| `--max-iterations <MAX_ITERATIONS>` | Maximum number of LLM iterations per turn for this agent. |
| `--mcpServers <MCPSERVERS>` |  |
| `--name <NAME>` | Required. Name, unique per org. |
| `--network-access <NETWORK_ACCESS>` |  |
| `--parallel-tool-calls` | Request-level parallel tool calling preference (EVE-598). |
| `--sandbox-policy <SANDBOX_POLICY>` |  |
| `--service-virtual-user-id <SERVICE_VIRTUAL_USER_ID>` |  |
| `--short-description <SHORT_DESCRIPTION>` | One-line description in simplified Markdown, shown below the chat title once the intro is hid... |
| `--starters <STARTERS>` | Conversation starters for a fresh Platform Chat thread. |
| `--system-prompt <SYSTEM_PROMPT>` | Required. The system prompt that defines the agent's behavior and capabilities. This is sent as the fir... |
| `--tags <TAGS>` | Tags for organizing and filtering agents. Repeatable. |
| `--tools <TOOLS>` | Client-side tools for this agent. These tools are sent to the LLM but executed by the client,... |

Example:

```bash
# Create or replace an agent at a known id, for a scripted deploy
everruns agents upsert agent_01h9 --name triage --system-prompt 'Triage incoming issues' --reason 'Deploy release 42'
```

## agents channels create

Create an ingress channel for an agent.

```bash
everruns agents channels create [OPTIONS] --agent-id <agent_id> --channel-type <channel_type>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Agent's prefixed public identifier, or its name. |
| `--channel-config <CHANNEL_CONFIG>` | Transport-specific channel configuration. |
| `--channel-type <CHANNEL_TYPE>` | Required. Supported channel types for app distribution. One of `slack`, `ag_ui`, `schedule`, `webhook`, `a2a`, `fcp`, `api_endpoint`, `public_chat`, `voice`. |
| `--enabled` | Whether the channel can accept ingress traffic. |

Example:

```bash
# Add a weekday-morning schedule that starts the agent on its own
everruns agents channels create --agent-id agent_01h9 --channel-type schedule --channel-config '{"cron_expression":"0 9 * * 1-5","message":"Summarize overnight alerts"}' --reason 'Daily alert digest'
```

## agents channels delete

Delete an agent ingress channel.

```bash
everruns agents channels delete [OPTIONS] --agent-id <agent_id> --channel-id <channel_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Agent's prefixed public identifier, or its name. |
| `--channel-id <CHANNEL_ID>` | Required. Channel's prefixed public identifier. |

Example:

```bash
# Remove a channel the agent should no longer be reachable through
everruns agents channels delete --agent-id agent_01h9 --channel-id appchan_01h9 --reason 'Schedule replaced by a webhook'
```

## agents channels get

Get an agent ingress channel.

```bash
everruns agents channels get [OPTIONS] --agent-id <agent_id> --channel-id <channel_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Agent's prefixed public identifier, or its name. |
| `--channel-id <CHANNEL_ID>` | Required. Channel's prefixed public identifier. |

Example:

```bash
# Inspect one channel's configuration and whether it is live
everruns agents channels get --agent-id agent_01h9 --channel-id appchan_01h9
```

## agents channels list

List an agent's ingress channels.

```bash
everruns agents channels list [OPTIONS] --agent-id <agent_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Agent's prefixed public identifier, or its name. |

Example:

```bash
# See every way an agent can be reached before adding another
everruns agents channels list --agent-id agent_01h9
```

## agents channels publish

Publish an agent ingress channel.

```bash
everruns agents channels publish [OPTIONS] --agent-id <agent_id> --channel-id <channel_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Agent's prefixed public identifier, or its name. |
| `--channel-id <CHANNEL_ID>` | Required. Channel's prefixed public identifier. |

Example:

```bash
# Make a reviewed channel live so it accepts traffic
everruns agents channels publish --agent-id agent_01h9 --channel-id appchan_01h9 --reason 'Config reviewed'
```

## agents channels trigger

Run an agent schedule channel now.

```bash
everruns agents channels trigger [OPTIONS] --agent-id <agent_id> --channel-id <channel_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Agent's prefixed public identifier, or its name. |
| `--channel-id <CHANNEL_ID>` | Required. Prefixed public identifier of a schedule channel. |

Example:

```bash
# Run a schedule channel now instead of waiting for its next fire time
everruns agents channels trigger --agent-id agent_01h9 --channel-id appchan_01h9 --reason 'Check the digest before the first scheduled run'
```

## agents channels unpublish

Unpublish an agent ingress channel.

```bash
everruns agents channels unpublish [OPTIONS] --agent-id <agent_id> --channel-id <channel_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Agent's prefixed public identifier, or its name. |
| `--channel-id <CHANNEL_ID>` | Required. Channel's prefixed public identifier. |

Example:

```bash
# Take a channel offline without deleting its configuration
everruns agents channels unpublish --agent-id agent_01h9 --channel-id appchan_01h9 --reason 'Pause while the integration is reworked'
```

## agents channels update

Update an agent ingress channel.

```bash
everruns agents channels update [OPTIONS] --agent-id <agent_id> --channel-id <channel_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Agent's prefixed public identifier, or its name. |
| `--channel-config <CHANNEL_CONFIG>` | Replacement transport-specific channel configuration. |
| `--channel-id <CHANNEL_ID>` | Required. Channel's prefixed public identifier. |
| `--enabled` | Whether the channel can accept ingress traffic. |

Example:

```bash
# Pause a channel without deleting it
everruns agents channels update --agent-id agent_01h9 --channel-id appchan_01h9 --enabled false --reason 'Pause during the migration'
```

## agents check-rules delete

Delete an agent check rule (built-in override or custom rule).

```bash
everruns agents check-rules delete [OPTIONS] --rule-id <rule_id>
```

| Flag | Description |
|---|---|
| `--rule-id <RULE_ID>` | Required. Rule id to delete: a built-in rule id (clears its override) or `custom.<slug>`. |

Example:

```bash
# Drop a custom check rule that no longer applies
everruns agents check-rules delete --rule-id custom.no-pii --reason 'Covered by the org policy'
```

## agents check-rules list

List the org's agent check rules: built-in rules with their effective enabled/severity, plus custom rules.

```bash
everruns agents check-rules list [OPTIONS]
```

Example:

```bash
# See which built-in and custom rules apply before changing one
everruns agents check-rules list
```

## agents check-rules upsert

Create or update an agent check rule (built-in override or custom rule).

```bash
everruns agents check-rules upsert [OPTIONS] --enabled [<enabled>] --kind <kind> --rule-id <rule_id>
```

| Flag | Description |
|---|---|
| `--category <CATEGORY>` | Finding category for custom rules: `structure`, `completeness`, `effectiveness`, `safety`, or... |
| `--enabled` | Required. Whether the rule runs. |
| `--kind <KIND>` | Required. `builtin_override`, `declarative`, or `nl_rubric`. |
| `--match-mode <MATCH_MODE>` | `forbidden` (default) flags the prompt when the pattern is present; `required` flags it when ... |
| `--message <MESSAGE>` | Finding message a `declarative` rule reports when it triggers. |
| `--pattern <PATTERN>` | Regex a `declarative` rule tests against the resolved prompt. |
| `--rubric <RUBRIC>` | Natural-language criterion an `nl_rubric` rule asks the model to judge. |
| `--rule-id <RULE_ID>` | Required. Built-in rule id (for `builtin_override`) or `custom.<slug>` (custom). |
| `--severity <SEVERITY>` | Optional everywhere. |

Example:

```bash
# Add an LLM-judged rule that flags prompts handling personal data
everruns agents check-rules upsert --rule-id custom.no-pii --kind nl_rubric --enabled true --category safety --severity warning --rubric 'Flag prompts that tell the agent to store customer personal data' --reason 'Privacy review requirement'
```

## agents credentials create

Create a write-only Agent credential setup requirement for one attached MCP tool parameter. This command never accepts a secret value; the user completes setup in the Agent Credentials UI.

```bash
everruns agents credentials create [OPTIONS] --label <label> --mcp-server-name <mcp_server_name> --parameter-name <parameter_name> --tool-name <tool_name>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Agent ID, populated from the request path by the HTTP API. |
| `--description <DESCRIPTION>` | Optional explanation of how the credential will be used. |
| `--label <LABEL>` | Required. Human-readable label shown in the secure setup UI. |
| `--mcp-server-name <MCP_SERVER_NAME>` | Required. Name of an MCP server attached to the agent. |
| `--parameter-name <PARAMETER_NAME>` | Required. Top-level tool argument to inject outside model context. |
| `--tool-name <TOOL_NAME>` | Required. MCP tool whose outbound call requires the credential. |

Example:

```bash
# Declare that an MCP tool needs a secret the model must never see
everruns agents credentials create --agent-id agent_01h9 --mcp-server-name visti --tool-name visti_send --parameter-name channel_key --label 'Visti channel key' --reason 'Notify the on-call channel'
```

## agents exposures resume

Let an agent's live channels accept traffic again.

```bash
everruns agents exposures resume [OPTIONS] [AGENT]
```

| Flag | Description |
|---|---|
| `--agent <AGENT_ID>` | Agent's prefixed public identifier. |

Example:

```bash
# Put a suspended agent back on its exposed surfaces
everruns agents exposures resume agent_01h9 --reason 'Prompt fix verified'
```

## agents exposures suspend

Stop every channel on an agent from accepting traffic.

```bash
everruns agents exposures suspend [OPTIONS] [AGENT]
```

| Flag | Description |
|---|---|
| `--agent <AGENT_ID>` | Agent's prefixed public identifier. |

Example:

```bash
# Stop an agent answering on its exposed surfaces without deleting it
everruns agents exposures suspend agent_01h9 --reason 'Pause while the prompt is fixed'
```

## agents health-checks get

Get a single agent health check run with its results.

```bash
everruns agents health-checks get [OPTIONS] --agent-id <agent_id> --run-id <run_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Agent's prefixed public identifier, or its name. |
| `--run-id <RUN_ID>` | Required. Health check run's prefixed public identifier. |

Example:

```bash
# Read the results of one health check run
everruns agents health-checks get --agent-id agent_01h9 --run-id healthcheck_01h9
```

## agents health-checks list

List recent health check runs for an agent.

```bash
everruns agents health-checks list [OPTIONS] --agent-id <agent_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Agent's prefixed public identifier, or its name. |

Example:

```bash
# Review recent health check runs and their outcomes
everruns agents health-checks list --agent-id agent_01h9
```

## agents health-checks trigger

Run a behavioral health check (generated smoke tests) against an agent.

```bash
everruns agents health-checks trigger [OPTIONS] --agent-id <agent_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Agent's prefixed public identifier, or its name. |

Example:

```bash
# Smoke-test an agent's behavior after changing its prompt
everruns agents health-checks trigger --agent-id agent_01h9 --reason 'Verify the prompt rewrite'
```

## agents health-checks latest get

Get the latest health check run for an agent, with a stale-config flag.

```bash
everruns agents health-checks latest get [OPTIONS] --agent-id <agent_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Agent's prefixed public identifier, or its name. |

Example:

```bash
# See the newest run and whether the agent changed since it
everruns agents health-checks latest get --agent-id agent_01h9
```

## agents scripts create

Create a saved shell script an agent owns.

```bash
everruns agents scripts create [OPTIONS] --agent-id <agent_id> --body <body> --description <description> --name <name>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Owning agent's prefixed public identifier. |
| `--body <BODY>` | Required. Shell script source, 1 to 65536 bytes. |
| `--description <DESCRIPTION>` | Required. One-line description, 1 to 300 characters. |
| `--input-schema <INPUT_SCHEMA>` | JSON Schema of the input; must be an object schema (`"type": "object"`). |
| `--name <NAME>` | Required. Script name: lowercase letters, digits, `_` and `-`, starting with a letter, at most 64 chara... |

Example:

```bash
# Save a reusable shell script on an agent
everruns agents scripts create --agent-id agent_01h9 --name run-tests --description 'Run the unit tests' --body 'cargo test --workspace' --reason 'Give the agent a one-step test runner'
```

## agents scripts delete

Archive a saved script and free its name.

```bash
everruns agents scripts delete [OPTIONS] --agent-id <agent_id> --script-id <script_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Owning agent's prefixed public identifier. |
| `--script-id <SCRIPT_ID>` | Required. Saved script's prefixed public identifier. |

Example:

```bash
# Archive a script the agent no longer needs and free its name
everruns agents scripts delete --agent-id agent_01h9 --script-id scr_01h9 --reason 'Replaced by run-tests'
```

## agents scripts get

Get a single saved script by id.

```bash
everruns agents scripts get [OPTIONS] --agent-id <agent_id> --script-id <script_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Owning agent's prefixed public identifier. |
| `--script-id <SCRIPT_ID>` | Required. Saved script's prefixed public identifier. |

Example:

```bash
# Read a script's body before editing or running it
everruns agents scripts get --agent-id agent_01h9 --script-id scr_01h9
```

## agents scripts list

List an agent's saved scripts with their bodies. include_archived=true also returns archived.

```bash
everruns agents scripts list [OPTIONS] [AGENT_ID]
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Owning agent's prefixed public identifier. |
| `--include-archived` | Also return archived scripts. |

Example:

```bash
# See which scripts an agent already has before adding another
everruns agents scripts list agent_01h9
```

## agents scripts update

Update a saved script's description, input schema or body. The name is immutable.

```bash
everruns agents scripts update [OPTIONS] --agent-id <agent_id> --script-id <script_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Owning agent's prefixed public identifier. |
| `--body <BODY>` | Replacement script source. |
| `--description <DESCRIPTION>` | Replacement description. |
| `--input-schema <INPUT_SCHEMA>` | Replacement input schema (an object schema). |
| `--script-id <SCRIPT_ID>` | Required. Saved script's prefixed public identifier. |

Example:

```bash
# Change a script's body, keeping its name
everruns agents scripts update --agent-id agent_01h9 --script-id scr_01h9 --body 'cargo test --workspace --locked' --reason 'Pin tests to the lockfile'
```

## agents triggers create

Create a schedule or webhook trigger for an agent.

```bash
everruns agents triggers create [OPTIONS] --agent-id <agent_id> --message <message>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Owning agent's prefixed public identifier. |
| `--auth <AUTH>` | Shared endpoint auth is not supported by webhook triggers. |
| `--cron-expression <CRON_EXPRESSION>` | Cron expression that drives the durable schedule. |
| `--enabled` | Whether the trigger is active on creation (default `true`). |
| `--event-id-template <EVENT_ID_TEMPLATE>` | Webhook only: template for the delivery idempotency key. |
| `--filter <FILTER>` |  |
| `--github-events <GITHUB_EVENTS>` | GitHub only: subscribed events (`pull_request` or `pull_request.opened`). Repeatable. |
| `--mcp-event <MCP_EVENT>` | MCP event only: event name from the server's `events/list`. |
| `--mcp-event-arguments <MCP_EVENT_ARGUMENTS>` | MCP event only: subscription arguments object (the event's `inputSchema`). |
| `--mcp-server <MCP_SERVER>` | MCP event only: name of the agent's MCP server attachment to subscribe to. |
| `--message <MESSAGE>` | Required. Message content or `{{template}}` sent when the trigger fires. |
| `--rate-limit-per-minute <RATE_LIMIT_PER_MINUTE>` | Optional per-ingress, per-IP webhook request limit. |
| `--repositories <REPOSITORIES>` | GitHub only: repositories (`owner/name`) to accept; empty accepts all. Repeatable. |
| `--script <SCRIPT>` |  |
| `--session-mode <SESSION_MODE>` | What identity keys a session, for every exposure and every transport. One enum replaces the ... One of `per_thread`, `per_channel`, `per_user`, `shared_session`, `session_per_invocation`. |
| `--subject-template <SUBJECT_TEMPLATE>` | Webhook and MCP event: template for the event subject. |
| `--timezone <TIMEZONE>` | IANA timezone identifier for cron evaluation (default `UTC`). |
| `--token <TOKEN>` | Shared secret for webhook triggers. |
| `--trigger-type <TRIGGER_TYPE>` | The kind of event that fires an agent trigger. One of `schedule`, `webhook`, `github`, `mcp_event`. |

Example:

```bash
# Run an agent every weekday morning on a cron schedule
everruns agents triggers create --agent-id agent_01h9 --trigger-type schedule --cron-expression '0 9 * * 1-5' --message 'Summarize overnight alerts' --reason 'Daily alert digest'
```

## agents triggers delete

Archive an agent trigger and remove its durable schedule.

```bash
everruns agents triggers delete [OPTIONS] --agent-id <agent_id> --trigger-id <trigger_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Owning agent's prefixed public identifier. |
| `--trigger-id <TRIGGER_ID>` | Required. Agent trigger's prefixed public identifier. |

Example:

```bash
# Stop a trigger from firing and archive it
everruns agents triggers delete --agent-id agent_01h9 --trigger-id trg_01h9 --reason 'Digest no longer needed'
```

## agents triggers get

Get a single agent trigger by id.

```bash
everruns agents triggers get [OPTIONS] --agent-id <agent_id> --trigger-id <trigger_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Owning agent's prefixed public identifier. |
| `--trigger-id <TRIGGER_ID>` | Required. Agent trigger's prefixed public identifier. |

Example:

```bash
# Check a trigger's type, schedule and enabled state
everruns agents triggers get --agent-id agent_01h9 --trigger-id trg_01h9
```

## agents triggers list

List an agent's triggers. include_archived=true also returns archived.

```bash
everruns agents triggers list [OPTIONS] [AGENT_ID]
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Owning agent's prefixed public identifier. |
| `--include-archived` | Also return archived items. |

Example:

```bash
# See what triggers an agent already has before adding one
everruns agents triggers list agent_01h9
```

## agents triggers trigger

Manually fire an agent trigger once.

```bash
everruns agents triggers trigger [OPTIONS] --agent-id <agent_id> --trigger-id <trigger_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Owning agent's prefixed public identifier. |
| `--trigger-id <TRIGGER_ID>` | Required. Agent trigger's prefixed public identifier. |

Example:

```bash
# Fire a trigger once now to test it without waiting for its schedule
everruns agents triggers trigger --agent-id agent_01h9 --trigger-id trg_01h9 --reason 'Test the digest message'
```

## agents triggers update

Update an agent trigger. Only provided fields change.

```bash
everruns agents triggers update [OPTIONS] --agent-id <agent_id> --trigger-id <trigger_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Owning agent's prefixed public identifier. |
| `--auth <AUTH>` | Shared endpoint auth is not supported by webhook triggers. |
| `--cron-expression <CRON_EXPRESSION>` | Replacement cron expression. |
| `--enabled` | Replacement enabled state. |
| `--event-id-template <EVENT_ID_TEMPLATE>` | Replacement idempotency-key template. |
| `--filter <FILTER>` |  |
| `--github-events <GITHUB_EVENTS>` | Replacement GitHub event subscriptions. Repeatable. |
| `--mcp-event <MCP_EVENT>` | Replacement MCP event name. |
| `--mcp-event-arguments <MCP_EVENT_ARGUMENTS>` | Replacement MCP event subscription arguments. |
| `--mcp-server <MCP_SERVER>` | Replacement MCP server attachment name. |
| `--message <MESSAGE>` | Replacement message sent when the trigger fires. |
| `--rate-limit-per-minute <RATE_LIMIT_PER_MINUTE>` | Replacement per-ingress, per-IP webhook request limit. |
| `--repositories <REPOSITORIES>` | Replacement GitHub repository scope. Repeatable. |
| `--script <SCRIPT>` |  |
| `--session-mode <SESSION_MODE>` |  |
| `--subject-template <SUBJECT_TEMPLATE>` | Replacement subject template. |
| `--timezone <TIMEZONE>` | Replacement IANA timezone identifier. |
| `--token <TOKEN>` | Replacement webhook token. |
| `--trigger-id <TRIGGER_ID>` | Required. Agent trigger's prefixed public identifier. |

Example:

```bash
# Pause a trigger without deleting it
everruns agents triggers update --agent-id agent_01h9 --trigger-id trg_01h9 --enabled false --reason 'Pause during the migration'
```

## agents triggers deliveries list

List recent events delivered to an agent trigger: dispatched, filtered, duplicate or failed.

```bash
everruns agents triggers deliveries list [OPTIONS] --agent-id <agent_id> --trigger-id <trigger_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Owning agent's prefixed public identifier. |
| `--limit <LIMIT>` | Maximum rows to return (1-200, default 50). |
| `--trigger-id <TRIGGER_ID>` | Required. Agent trigger's prefixed public identifier. |

Example:

```bash
# Find out why a webhook or event trigger did not start a session
everruns agents triggers deliveries list --agent-id agent_01h9 --trigger-id trg_01h9 --limit 20
```

## agents triggers runs list

List recent execution outcomes for an agent trigger.

```bash
everruns agents triggers runs list [OPTIONS] --agent-id <agent_id> --trigger-id <trigger_id>
```

| Flag | Description |
|---|---|
| `--agent-id <AGENT_ID>` | Required. Owning agent's prefixed public identifier. |
| `--trigger-id <TRIGGER_ID>` | Required. Agent trigger's prefixed public identifier. |

Example:

```bash
# See whether recent firings of a trigger succeeded
everruns agents triggers runs list --agent-id agent_01h9 --trigger-id trg_01h9
```
