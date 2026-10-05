---
title: CLI
description: Manage agents, sessions, and conversations from the command line.
sidebar:
  label: CLI
appliesTo: [platform, cloud]
---

The `everruns` CLI is a command-line client for the Everruns API. It covers the same surface as the SDK (agents, sessions, messages, capabilities) and is designed to compose well with shell pipelines.

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

The CLI defaults to [Everruns Cloud](https://app.everruns.com) at `https://app.everruns.com/api`. Sign in once:

```bash
everruns login          # opens the browser for OAuth
everruns login --token  # paste a personal access token instead (SSH, headless)
everruns status         # show the current user and organization
```

For a self-hosted deployment, point the CLI at your API first:

```bash
# Per command
everruns --api-url http://localhost:9300/api agents list

# Per shell
export EVERRUNS_API_URL=http://localhost:9300/api
export EVERRUNS_API_KEY=evr_pat_...   # or run `everruns login`
```

## Command surface

| Group | Subcommands |
|---|---|
| `login`, `logout`, `status` | Sign in, sign out, show the current user and organization |
| `orgs` | `list` organizations, `select` the active one |
| `agents` | `create`, `update`, `import`, `export`, `validate`, `diff`, plus the platform commands |
| `sessions` | `create`, `watch`, `export`, plus the platform commands |
| `chat` | Send a message and stream the response |
| `files` | Sync files between a local folder and a session |
| `connections` | `set`, `list`, `remove` your provider API keys |
| Every other platform command | See [Platform commands](#platform-commands) |

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
everruns agents get agent_...
everruns agents delete agent_...
```

### Sessions

```bash
everruns sessions create --agent agent_...
everruns sessions create --agent agent_... --title "Debug session"

# With session-level overrides
everruns sessions create \
  --agent agent_... \
  --harness worker-base \
  --capability 'web_fetch={"timeout":10}' \
  --hint setup_connection=true \
  --network-allow api.example.com \
  --max-iterations 8
```

Also accepts: `--locale`, repeatable `--tag`, `--system-prompt`, `--hints-json`, repeatable `--network-block`, repeatable `--secret KEY=VALUE`, and budget flags.

```bash
everruns sessions list
everruns sessions get session_...
```

### Chat

```bash
everruns chat "Tell me a joke!" --session session_...
```

Options: `--timeout <seconds>` (default 300), `--no-stream` to queue without waiting.

## Output formats

Every command accepts `-o` / `--output`:

```bash
everruns agents list -o json
everruns agents list -o yaml
```

`--quiet` suppresses headers and prints only the essential identifier, useful for capturing IDs in shell variables.

## Platform commands

Every other command comes from the same command contract that agents use in their shell and through MCP `execute`, so the spelling, flags and validation are identical everywhere. Run `everruns --help`, `everruns <noun> --help` or `everruns <noun> <verb> --help` to browse them.

```bash
everruns agents versions list --agent agent_...
everruns agents triggers list --agent-id agent_...
everruns sessions participants add --session-id session_... --kind agent --agent-id agent_...
everruns capabilities list --search web

# Plugins, skills and knowledge bases
everruns plugins install --marketplace-id <marketplace-id> --plugin-name <plugin-name>
everruns skills create --skill-md @./SKILL.md
everruns knowledge-bases create --name "Product docs" --description "Published product documentation"
```

These commands print the command's JSON output (YAML with `-o yaml`). A text or JSON flag written `@path` is read from that local file, as in `--skill-md @./SKILL.md`; write `@@` for a value that really starts with `@`.

## See also

- [Automate with the CLI](/how-to/automate-with-the-cli/): `jq`, quiet mode, scripting patterns.
- [Define agents as files](/how-to/define-agents-as-files/): file formats for `-f`.
- [SDK](/features/sdk/): the programmatic equivalent.
