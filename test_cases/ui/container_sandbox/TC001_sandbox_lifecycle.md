# TC001: Container Sandbox - Sandbox Lifecycle

## Description

Verify that the Coding (Container) harness creates a container sandbox, executes a command, and removes the sandbox on request.

## Preconditions

- Server running (`just start-all` -- full mode with PostgreSQL required for leased resources)
- User logged in
- LLM API keys configured (Anthropic or OpenAI)
- Docker Engine accessible from the server (local socket or remote TCP)
- `CONTAINER_SANDBOX_DOCKER_HOST` set if Docker is not on the default socket

## Test Data

| Field | Value |
|-------|-------|
| Harness | Coding (Container) (built-in, name: `coding-container`) |
| First Message | Create a sandbox and calculate the result of 123 * 456. Do NOT remove the sandbox after - I want to keep it running. |
| Cleanup Message | Remove the sandbox |

## Steps

1. Create a new session using the **Coding (Container)** harness
2. Send the message: `Create a sandbox and calculate the result of 123 * 456. Do NOT remove the sandbox after - I want to keep it running.`
3. Wait for the agent to create a sandbox (tool call: `sandbox_create`)
4. Wait for the agent to execute the calculation (tool call: `sandbox_exec`)
5. Verify the agent responds with the correct result (123 * 456 = 56088)
6. Send the message: `Remove the sandbox`
7. Wait for the agent to remove the sandbox (tool call: `sandbox_manage` with action "remove")
8. Verify the agent confirms the sandbox has been removed

## Notes

- The Coding (Container) harness system prompt says "Always delete sandboxes when done." The first message must instruct the agent **not** to remove the sandbox to test explicit removal as a separate step.
- **DEV_MODE limitation**: Leased resource tracking requires PostgreSQL. Use `just start-all` for reliable testing.
- Docker must be accessible from the server process. If running in a container, ensure Docker socket is mounted or a remote Docker host is configured.

## Expected Result

| Check | Expected |
|-------|----------|
| Sandbox created | Agent creates container successfully (container name returned) |
| Command executed | Agent runs calculation, result includes 56088 |
| Sandbox removed | Agent confirms container and network are removed |
| Resources cleaned | Session resources page shows the sandbox as "Released" |

## Cleanup

- If the sandbox removal step failed, check `docker ps -a` for orphaned containers with `managed-by=everruns` label and remove them manually
- Leased resource scheduler will auto-cleanup after 20 minutes
