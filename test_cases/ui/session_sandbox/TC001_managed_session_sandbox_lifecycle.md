# TC001: Managed Session Sandbox - Lifecycle

## Description

Verify that the built-in **Coding (Session Sandbox)** harness uses the provider-neutral managed sandbox flow end to end: create or resume a single session-owned sandbox, execute work through `sandbox_*` tools, pause after idle, and auto-resume on the next sandbox tool call.

## Preconditions

- Server running in full mode (`just start-all`)
- `FEATURE_SESSION_SANDBOX=true`
- Secrets encryption configured (`SECRETS_ENCRYPTION_KEY`)
- User logged in
- LLM API keys configured (Anthropic or OpenAI)
- Valid Daytona connection available in **Settings > Connections**

## Test Data

| Field | Value |
|-------|-------|
| Harness | Coding (Session Sandbox) (built-in, name: `coding-session-sandbox`) |
| First Message | Calculate `123 * 456`, keep the sandbox alive, and tell me the working directory. |
| Resume Message | Run `pwd` again in the sandbox. |

## Steps

1. Create a new session using the **Coding (Session Sandbox)** harness
2. Send the message: `Calculate 123 * 456, keep the sandbox alive, and tell me the working directory.`
3. Verify the run uses `sandbox_exec` / `sandbox_status` and does **not** use raw provider tools such as `daytona_exec` or local shell tools such as `bash`
4. Verify the reply contains `56088`
5. Open the session resources page and verify one sandbox resource is present
6. Wait at least 3 minutes for the session to idle
7. Verify the sandbox is shown as paused or stopped in the session resources view (or via `sandbox_status` in the transcript)
8. Send the message: `Run pwd again in the sandbox.`
9. Verify the next sandbox tool call succeeds without manual sandbox creation and returns the sandbox working directory
10. Verify the same session continues using provider-neutral `sandbox_*` tools

## Expected Result

| Check | Expected |
|-------|----------|
| Managed tools only | Transcript shows `sandbox_*` tools rather than `daytona_*` or `bash` |
| Command execution | Agent returns `56088` for the first request |
| One sandbox per session | Session resources shows a single sandbox resource for the session |
| Idle pause | Sandbox pauses after session idle timeout |
| Auto-resume | Next sandbox tool call resumes the sandbox automatically |

## Cleanup

- Delete the sandbox via `sandbox_manage` with action `delete`, or end the session and allow normal cleanup
