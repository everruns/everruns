---
title: CLI
description: The everruns CLI manages agents, sessions, and conversations from the command line. Useful for scripting, automation, and quick checks without the UI.
sidebar:
  label: CLI
---

The `everruns` CLI is a command-line client for the Everruns API. It covers the same surface as the SDK, agents, sessions, messages, capabilities, and is designed to compose well with shell pipelines.

This page covers installation, configuration, and the command surface. For scripting patterns and `jq` examples, see [Automate with the CLI](/how-to/automate-with-the-cli/).

## Install

### Homebrew (macOS / Linux)

```bash
brew tap everruns/tap
brew install everruns
```

### Cargo

From the Git repository:

```bash
cargo install --git https://github.com/everruns/everruns everruns-cli
```

Or clone and build:

```bash
git clone https://github.com/everruns/everruns.git
cd everruns
cargo install --path crates/cli
```

### Verify

```bash
everruns --version
```

## Configure

The CLI defaults to the hosted API at `https://app.everruns.com/api`. Override for local or self-hosted deployments:

```bash
# Per command
everruns --api-url http://localhost:9300/api agents list

# Per shell
export EVERRUNS_API_URL=http://localhost:9300/api
export EVERRUNS_API_KEY=dev
```

## Command surface

| Group | Subcommands |
|---|---|
| `agents` | `create`, `list`, `get`, `update`, `delete` |
| `sessions` | `create`, `list`, `get`, `cancel`, `delete` |
| `capabilities` | (list, no subcommand) |
| `chat` | Send a message and stream the response |

### Agents

```bash
# Inline
everruns agents create \
  --name "my-agent" \
  --system-prompt "You are a helpful assistant." \
  --tag production

# From a file (TOML, YAML, JSON, or Markdown front matter)
everruns agents create -f agent.toml
everruns agents create -f agent.yaml
everruns agents create -f agent.md
```

If `./agent.toml` exists and you don't pass inline flags, `everruns agents create` picks it up automatically. The file formats are documented in [Define agents as files](/how-to/define-agents-as-files/).

```bash
everruns agents list
everruns agents get agt_xxx
everruns agents delete agt_xxx
```

### Sessions

```bash
everruns sessions create --agent agt_xxx
everruns sessions create --agent agt_xxx --title "Debug session"

# With session-level overrides
everruns sessions create \
  --agent agt_xxx \
  --harness generic \
  --capability 'web_fetch={"timeout":10}' \
  --hint setup_connection=true \
  --network-allow api.example.com \
  --max-iterations 8
```

Also accepts: `--locale`, repeatable `--tag`, `--system-prompt`, `--hints-json`, repeatable `--network-block`, repeatable `--secret KEY=VALUE`, and budget flags.

```bash
everruns sessions list
everruns sessions get ses_xxx
```

### Chat

```bash
everruns chat "Tell me a joke!" --session ses_xxx
```

Options: `--timeout <seconds>` (default 300), `--no-stream` to queue without waiting.

## Output formats

Every command accepts `-o` / `--output`:

```bash
everruns agents list -o json
everruns agents list -o yaml
```

`--quiet` suppresses headers and prints only the essential identifier, useful for capturing IDs in shell variables.

## See also

- [Automate with the CLI](/how-to/automate-with-the-cli/), `jq`, quiet mode, scripting patterns.
- [Define agents as files](/how-to/define-agents-as-files/), file formats for `-f`.
- [SDK](/features/sdk/), the programmatic equivalent.

## Agent composition

Discover and manage the plugins, skills, and knowledge bases available to agents:

```bash
# Plugins
everruns plugins list
everruns plugins get <plugin-id>
everruns plugins install <marketplace-id> <plugin-name>
everruns plugins uninstall <plugin-id>

# Skills
everruns skills list
everruns skills get <skill-id>
everruns skills create ./SKILL.md
everruns skills delete <skill-id>

# Knowledge bases
everruns knowledge-bases list
everruns knowledge-bases get <knowledge-base-id>
everruns knowledge-bases create "Product docs" --description "Published product documentation"
everruns knowledge-bases delete <knowledge-base-id>
```

Resource identifiers are URL-encoded before requests are sent. Use the global `--output json` or `--output yaml` option for machine-readable discovery output.

Skill creation reads the supplied Markdown file and sends its contents to Everruns. Knowledge document ingestion and assigning composition resources to an agent are not yet exposed by the CLI.
