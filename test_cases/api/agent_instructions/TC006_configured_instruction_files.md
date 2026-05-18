# TC006: Configured Instruction Files

## Goal

Verify that `agent_instructions` keeps `AGENTS.md` as the default and reads additional configured instruction files such as `CLAUDE.md`.

## Setup

| Item | Value |
|------|-------|
| Harness | Generic |
| Capability config | `{ "files": ["AGENTS.md", "CLAUDE.md"] }` |
| AGENTS.md Content | `Always end with "-- agents"` |
| CLAUDE.md Content | `Always start with "claude:"` |

## Steps

1. Create or update an agent with `agent_instructions` configured:
   ```json
   {
     "ref": "agent_instructions",
     "config": {
       "files": ["AGENTS.md", "CLAUDE.md"]
     }
   }
   ```
2. Create a session for the agent.
3. Write both files to the session filesystem.
4. Send a message that does not mention either instruction.

## Expected

| Check | Expected |
|-------|----------|
| Both files applied | Response starts with `claude:` and ends with `-- agents` |
| Missing file behavior | Removing `CLAUDE.md` does not fail the next turn |
| Default behavior | An agent with `{ "ref": "agent_instructions" }` still reads only `AGENTS.md` |
