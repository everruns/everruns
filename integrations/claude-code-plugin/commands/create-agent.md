---
description: Create a new Everruns agent
argument-hint: "<name> [--model <model_id>] [--harness <harness_id>] [--instructions \"...\"]"
---

Create a new agent on Everruns via the MCP `execute` tool invoking `create_agent`.

Arguments: `$ARGUMENTS`

Steps:
1. Parse a name (required) and optional `--model`, `--harness`, `--instructions` flags.
2. If the user did not specify a model, call `list_models` first and let them pick — do not silently guess.
3. If the user did not specify a harness, call `list_harnesses` and default to the org base harness unless the user prefers another.
4. Call `create_agent --name "<name>" [--model_id <id>] [--harness_id <id>] [--instructions "..."]`.
5. On success, print the new agent id and a one-line summary. Suggest `/everruns:agent-run <agent_id> "<message>"` as the next step.

Do not invent ids. If any required parameter is missing after asking the user, stop and report what is needed.
