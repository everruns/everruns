# TC001: Secure MCP Credential Lifecycle

## Description

Verify write-only Agent credential create, rotate, and revoke behavior for a
bound MCP tool parameter.

## Preconditions

- Canonical local stack is running with `AUTH_MODE=none`.
- An Agent has a controlled MCP server attached and a pending credential binding.
- Use a unique disposable sentinel; never use a real external credential.

## Steps

1. Open the Agent and select **Credentials**.
2. Confirm the pending binding shows only server, tool, parameter, label, and
   **Setup required**. Inspect the DOM and network response: no value field is
   present in returned JSON.
3. Enter the disposable sentinel in the masked value input and save.
4. Confirm the input clears and unmounts, the binding says **Configured**, and
   the sentinel is absent from DOM text, browser history, console, network
   response, and screenshot.
5. Choose **Rotate**, enter a second disposable sentinel, and save. Confirm the
   same non-disclosure properties.
6. Revoke the binding and confirm deletion. Invoke the tool and verify its
   structured result contains `credential_required` plus the Agent Credentials
   setup URL, but neither sentinel.
7. Try to replace the deleted binding and a binding belonging to another Agent;
   both requests must return not found and reveal no metadata.

## Expected Result

- Values are write-only, masked, cleared after submission, and never read back.
- Rotation changes future calls immediately.
- Revocation prevents future injection and produces a safe direct setup path.
- Binding access is limited to the owning organization and Agent.
