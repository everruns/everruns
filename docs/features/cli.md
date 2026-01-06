---
title: CLI
description: Command-line interface for managing agents, sessions, and conversations
---

The `everruns` CLI provides a command-line interface for managing agents, sessions, and conversations. It's useful for scripting, automation, and quick interactions without using the web UI.

## Installation

Install from the Git repository using Cargo:

```bash
cargo install --git https://github.com/everruns/everruns everruns-cli
```

Or clone and build locally:

```bash
git clone https://github.com/everruns/everruns.git
cd everruns
cargo install --path crates/cli
```

Verify the installation:

```bash
everruns --version
```

## Configuration

The CLI connects to the Everruns API. Configure the API URL:

```bash
# Via command-line flag
everruns --api-url http://localhost:9000 agents list

# Via environment variable
export EVERRUNS_API_URL=http://localhost:9000
everruns agents list
```

Default: `http://localhost:9000`

## Commands

### Agents

Manage agent configurations.

#### Create Agent

```bash
# Inline creation
everruns agents create \
  --name "my-agent" \
  --system-prompt "You are a helpful assistant." \
  --capability current_time \
  --capability web_fetch \
  --tag production

# From YAML file
everruns agents create -f agent.yaml

# From JSON file
everruns agents create -f agent.json

# From Markdown with front matter
everruns agents create -f agent.md
```

**YAML file format** (`agent.yaml`):

```yaml
name: "research-assistant"
description: "Helps with research tasks"
system_prompt: |
  You are a helpful research assistant.
  Always cite your sources.
capabilities:
  - current_time
  - web_fetch
tags:
  - research
  - assistant
```

**Markdown file format** (`agent.md`):

```markdown
---
name: "research-assistant"
description: "Helps with research tasks"
capabilities:
  - current_time
  - web_fetch
tags:
  - research
---
You are a helpful research assistant.

Always cite your sources and provide accurate information.
```

The markdown body becomes the system prompt.

#### List Agents

```bash
everruns agents list
```

Output:

```
ID                                    NAME              STATUS   CAPABILITIES
550e8400-e29b-41d4-a716-446655440000  research-bot      active   current_time, web_fetch
660e8400-e29b-41d4-a716-446655440001  joke-bot          active   -
```

#### Get Agent

```bash
everruns agents get <agent-id>
```

#### Delete Agent

```bash
everruns agents delete <agent-id>
```

### Capabilities

List available capabilities that can be assigned to agents.

```bash
# List available capabilities
everruns capabilities

# List all including coming soon
everruns capabilities --status all

# List only coming soon
everruns capabilities --status coming_soon
```

Output:

```
ID                    NAME               STATUS      CATEGORY
current_time          Current Time       available   Utilities
web_fetch             Web Fetch          available   Network
session_file_system   File System        available   File Operations
stateless_todo_list   Task Management    available   Productivity
```

### Sessions

Manage conversation sessions for an agent.

#### Create Session

```bash
everruns sessions create --agent <agent-id>

# With title
everruns sessions create --agent <agent-id> --title "Debug session"
```

#### List Sessions

```bash
everruns sessions list --agent <agent-id>
```

#### Get Session

```bash
everruns sessions get --agent <agent-id> --session <session-id>
```

### Chat

Send a message and receive the agent's response.

```bash
everruns chat "Tell me a joke!" --session <session-id> --agent <agent-id>
```

Output:

```
You: Tell me a joke!

Agent: Why don't scientists trust atoms? Because they make up everything!
```

Options:

- `--timeout <seconds>` - Max wait time for response (default: 300)
- `--no-stream` - Send message and exit without waiting for response

## Output Formats

The CLI supports multiple output formats for scripting:

```bash
# Default text format
everruns agents list

# JSON format
everruns agents list --output json

# YAML format
everruns agents list --output yaml
```

## Quiet Mode

Suppress non-essential output for scripting:

```bash
# Only output the created agent ID
everruns agents create -f agent.yaml --quiet
# Output: 550e8400-e29b-41d4-a716-446655440000

# Use in scripts
AGENT_ID=$(everruns agents create -f agent.yaml -q)
SESSION_ID=$(everruns sessions create --agent $AGENT_ID -q)
everruns chat "Hello!" --session $SESSION_ID --agent $AGENT_ID
```

## Examples

### Complete Workflow

```bash
# 1. Create an agent
everruns agents create \
  --name "assistant" \
  --system-prompt "You are a helpful assistant." \
  --capability current_time \
  --quiet > agent_id.txt

AGENT_ID=$(cat agent_id.txt)

# 2. Create a session
SESSION_ID=$(everruns sessions create --agent $AGENT_ID -q)

# 3. Chat with the agent
everruns chat "What time is it?" --session $SESSION_ID --agent $AGENT_ID
```

### Using Agent Files

```bash
# Create agent.md
cat > agent.md << 'EOF'
---
name: "code-reviewer"
description: "Reviews code and suggests improvements"
capabilities:
  - current_time
tags:
  - development
---
You are an expert code reviewer.

When reviewing code:
1. Check for bugs and edge cases
2. Suggest performance improvements
3. Ensure code follows best practices
EOF

# Create the agent
everruns agents create -f agent.md
```

### JSON Output for Scripting

```bash
# Get agent details as JSON and extract with jq
everruns agents get <agent-id> --output json | jq '.capabilities'

# List agents and filter
everruns agents list --output json | jq '.data[] | select(.status == "active")'
```
