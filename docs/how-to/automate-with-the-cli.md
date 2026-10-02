---
title: Automate with the CLI
description: Script the Everruns CLI with structured output, jq, quiet mode, and shell pipelines for CI, cron jobs, and integration with other tools.
---

The CLI emits structured output (JSON, YAML) for scripting. Combined with `jq` and `--quiet` mode, it composes naturally with shell pipelines.

For a command reference, see [CLI](/features/cli/).

## Capture IDs

```bash
AGENT_ID=$(everruns agents create \
  --name "assistant" \
  --system-prompt "You are a helpful assistant." \
  -o json | jq -r '.id')

SESSION_ID=$(everruns sessions create --agent "$AGENT_ID" -o json | jq -r '.id')

everruns chat "What time is it?" --session "$SESSION_ID"
```

## Quiet mode

`--quiet` suppresses headers and tables, printing only the essential identifier:

```bash
everruns agents create -f agent.toml --quiet
# Output: agent_550e8400e29b41d4a716446655440000
```

Useful inside `$(...)` substitution when JSON parsing is overkill.

## Filter listings

```bash
# Active agents only
everruns agents list --output json | jq '.data[] | select(.status == "active")'

# Just the names of agents tagged "production"
everruns agents list -o json \
  | jq -r '.data[] | select(.tags[]? == "production") | .name'

# Agents created in the last 24h
everruns agents list -o json \
  | jq --arg cutoff "$(date -u -d '24 hours ago' +%FT%TZ)" \
       '.data[] | select(.created_at > $cutoff)'
```

## Authenticate and configure the API URL

The CLI targets Everruns Cloud (`https://app.everruns.com/api`) by default. Sign in once:

```bash
everruns login           # browser sign-in
everruns login --token   # headless/SSH: paste an API key
```

For a self-hosted server, point the CLI at it:

```bash
# Per-command
everruns --api-url http://localhost:9300/api agents list

# For the whole shell
export EVERRUNS_API_URL=http://localhost:9300/api
```

In CI, skip `login`: set `EVERRUNS_API_KEY` (and `EVERRUNS_API_URL` for self-hosted) from secrets and the CLI picks them up. `EVERRUNS_API_KEY` always takes precedence over stored credentials.

## Drive sessions from a file-defined agent

```bash
cat > agent.md <<'EOF'
---
name: "code-reviewer"
capabilities:
  - ref: current_time
  - ref: session_file_system
tags: [development]
---
You are an expert code reviewer.

When reviewing code:
1. Check for bugs and edge cases
2. Suggest performance improvements
3. Ensure code follows best practices
EOF

AGENT_ID=$(everruns agents create -f agent.md -o json | jq -r '.id')
SESSION_ID=$(everruns sessions create --agent "$AGENT_ID" -o json | jq -r '.id')

everruns chat "Review the diff at HEAD~1..HEAD" --session "$SESSION_ID"
```

## Send-and-exit (no streaming)

`--no-stream` queues the message and returns immediately. Useful when a downstream system polls for results.

```bash
everruns chat "Process the queue" --session "$SESSION_ID" --no-stream
```

## See also

- [CLI reference](/features/cli/), full command list and flags.
- [Define agents as files](/how-to/define-agents-as-files/), the file formats accepted by `-f`.
