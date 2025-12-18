# Capabilities

Capabilities are modular functionality units that extend Agent behavior. A Capability can contribute additions to the system prompt, provide tools, and modify execution behavior. Users can enable multiple capabilities on an Agent to compose functionality.

## Overview

When you assign capabilities to an agent, those capabilities enhance what the agent can do:

- **System Prompt Additions**: Some capabilities add instructions to the agent's system prompt
- **Tools**: Capabilities can provide tools that the agent can use during conversations
- **Behavior Modifications**: Future capabilities may modify how the agent processes requests

## Available Capabilities

### Current Time

**Status**: Available

Adds a tool to get the current date and time in various formats and timezones.

- **Tool**: `get_current_time`
- **Formats**: ISO 8601, Unix timestamp, human-readable
- **Use cases**: Agents that need to know the current time or date

### No-Op

**Status**: Available

A no-operation capability for testing and demonstration purposes. Does not add any functionality.

- **Use cases**: Testing capability assignment workflow

### Deep Research

**Status**: Coming Soon

Enables deep research capabilities with a scratchpad for notes, web search tools, and structured thinking.

- **System Prompt**: Adds research scratchpad instructions
- **Use cases**: Research-focused agents

### Sandboxed Execution

**Status**: Coming Soon

Enables sandboxed code execution environment for running code safely.

- **System Prompt**: Adds code execution instructions
- **Use cases**: Agents that need to run code

### File System Access

**Status**: Coming Soon

Adds tools to access and manipulate files - read, write, grep, and more.

- **System Prompt**: Adds file system access instructions
- **Use cases**: Agents that need to work with files

## Managing Capabilities

### Via UI

1. Navigate to the Agent detail page
2. Find the **Capabilities** section in the sidebar
3. Enable or disable capabilities using the checkboxes
4. Reorder capabilities using the up/down arrows
5. Click **Save** to apply changes

The order of capabilities matters - capabilities are applied in the order shown, with earlier capabilities' system prompt additions appearing first.

### Via API

Get agent capabilities:

```bash
curl -X GET http://localhost:9000/v1/agents/{agent_id}/capabilities
```

Set agent capabilities (order determines priority):

```bash
curl -X PUT http://localhost:9000/v1/agents/{agent_id}/capabilities \
  -H "Content-Type: application/json" \
  -d '{
    "capabilities": ["current_time", "noop"]
  }'
```

List all available capabilities:

```bash
curl -X GET http://localhost:9000/v1/capabilities
```

## Capability Application Flow

When a session runs, capabilities are applied as follows:

1. The agent's assigned capabilities are fetched (ordered by position)
2. Each capability's system prompt addition is collected
3. Each capability's tools are collected
4. The final system prompt = capability additions + agent's base prompt
5. All capability tools are made available to the agent

## Best Practices

1. **Order Matters**: Place more important capabilities first - their instructions appear earlier in the system prompt
2. **Minimal Capabilities**: Only enable capabilities the agent actually needs
3. **Test Combinations**: Some capability combinations may produce unexpected behaviors
4. **Check Status**: Coming Soon capabilities cannot be enabled yet
