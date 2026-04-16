# Coding Session Sandbox Harness

Built-in coding harness using the managed `session_sandbox` capability.

## Design

**Name:** `coding-session-sandbox`  
**Display Name:** Coding (Session Sandbox)  
**Parent:** `generic`  
**Additional capability:** `session_sandbox` configured with provider `daytona`

This harness exists as the canonical example for the experimental managed
sandbox flow. It demonstrates the intended user experience:

- one sandbox per session
- provider-neutral `sandbox_*` tools
- auto-start and auto-resume handled by the system
- idle pause after 3 minutes

## Effective stack

Inherited from `generic`:

- workspace filesystem
- virtual bash
- web fetch
- session storage
- AGENTS.md support
- skills
- infinity context
- budgeting / compaction / output persistence

Added by this harness:

- `session_sandbox` with Daytona provider configuration

## Prompt intent

The harness prompt steers coding work into the managed sandbox and avoids raw
provider tools. It keeps the workflow close to `coding-daytona`, but removes
manual sandbox creation and sandbox selection from the model’s job.
