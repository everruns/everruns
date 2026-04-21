# Coding Session Sandbox Harness

Built-in coding harness using the managed `session_sandbox` capability.

## Design

**Name:** `coding-session-sandbox`  
**Display Name:** Coding (Session Sandbox)  
**Additional capability:** `session_sandbox` configured with provider `daytona`

This harness exists as the canonical example for the experimental managed
sandbox flow. It demonstrates the intended user experience:

- one sandbox per session
- provider-neutral `sandbox_*` tools
- auto-start and auto-resume handled by the system
- idle pause after 3 minutes
- no local shell escape hatch; command execution goes through `sandbox_exec`

## Effective stack

- session filesystem
- web fetch
- session storage
- session metadata / schedules
- AGENTS.md support
- skills
- infinity context
- budgeting / compaction / output persistence
- `session_sandbox` with Daytona provider configuration

Intentionally omitted compared with `generic`:

- `virtual_bash`

## Prompt intent

The harness prompt steers coding work into the managed sandbox and avoids raw
provider tools. It keeps the workflow close to `coding-daytona`, removes
manual sandbox creation and sandbox selection from the model’s job, and makes
the managed sandbox the only shell execution path.
