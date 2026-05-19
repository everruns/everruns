# TC001: Fake AWS Configured Agent Handoff

## Description

Verify that a source agent with the `agent_handoff` capability can delegate an
AWS-style request to a configured target Agent only after the user has a
required Fake AWS connection. The source agent must not receive Fake AWS tools
or credentials directly. The target agent must run in a child session, the
handoff must be registered as a session resource, and follow-up messages must be
accepted only for child sessions owned by the source session.

## Preconditions

- Control-plane running (`PORT_PREFIX=271 just start-dev --no-watch` or
  equivalent)
- The active org has the built-in `generic` harness
- The active org has a usable default model, or the agents are configured with a
  deterministic `llmsim` model for local execution
- The signed-in/dev user has no existing `fake_aws` connection at the start of
  the negative-path check

## Test Data

| Field | Value |
|-------|-------|
| Target agent name | `aws-operator-handoff-tc001` |
| Target agent display name | `AWS Operator Handoff TC001` |
| Target capabilities | `fake_aws` |
| Source agent name | `welcome-handoff-tc001` |
| Source agent display name | `Welcome Handoff TC001` |
| Source capabilities | `agent_handoff` |
| Handoff target id | `aws_operator` |
| Required connection | `fake_aws` |
| Required scope label | `fake_aws:rds:create` |
| Fake AWS key | `fake_aws_tc001_key` |
| User request | `Create an RDS database named app-db in us-east-1.` |

## Steps

1. Create the target `AWS Operator Handoff TC001` agent on the `generic`
   harness with the `fake_aws` capability and a system prompt that instructs it
   to use Fake AWS tools for AWS infrastructure requests.

2. Create the source `Welcome Handoff TC001` agent on the `generic` harness
   with the `agent_handoff` capability configured with this target:

   ```json
   {
     "targets": [
       {
         "id": "aws_operator",
         "name": "AWS Operator",
         "agent_id": "<TARGET_AGENT_ID>",
         "required_connections": ["fake_aws"],
         "required_scopes": ["fake_aws:rds:create"]
       }
     ]
   }
   ```

3. Start a session with the source agent.

4. Send the user request:

   ```text
   Create an RDS database named app-db in us-east-1.
   ```

5. Verify the first run requests connection setup instead of handing work off.

6. Add a Fake AWS user connection through Settings > Connections or the API:

   ```bash
   curl -s -X POST "http://localhost:27100/api/v1/user/connections/fake_aws" \
     -H "Content-Type: application/json" \
     -d '{"api_key":"fake_aws_tc001_key"}'
   ```

7. Retry the user request in the same source session.

8. Wait for the source session to idle.

9. Inspect source session events and messages.

10. Inspect session resources for the source session.

11. Inspect the child session returned by the handoff resource.

12. Send a follow-up through `message_agent_handoff`, for example:

    ```text
    List the RDS databases you can see now.
    ```

## Expected Result

### Missing Connection Gate

| Check | Expected |
|-------|----------|
| Missing connection result | Source turn includes a `connection_required` result for provider `fake_aws` |
| No child session before connection | No `agent_handoff` session resource is registered |
| No credential prompt | Source agent does not ask the user to paste an API key into chat |

### Successful Handoff

| Check | Expected |
|-------|----------|
| Handoff tool called | Source session includes a `start_agent_handoff` tool call |
| Target allowlist enforced | Tool arguments use `target: "aws_operator"` |
| Child session created | A child session exists with `parent_session_id` equal to the source session id |
| Target agent selected | Child session `agent_id` equals `<TARGET_AGENT_ID>` |
| Handoff resource registered | Source session has `session_resources.kind = "agent_handoff"` |
| Resource id | Handoff resource `resource_id` equals the child session id |
| Resource status | Handoff resource transitions from `active` to `completed` or `failed` |
| Resource metadata | Metadata includes target id, target agent id, required connection ids, required scope labels, and mode |
| Secret exclusion | Resource metadata does not include `fake_aws_tc001_key`, request credential text, or full task text |
| Target tool ownership | Child session can call Fake AWS tools; source session does not receive Fake AWS tool definitions from the target |

### Follow-up

| Check | Expected |
|-------|----------|
| Follow-up ownership | `message_agent_handoff` accepts the child handoff id from this source session |
| Cross-session guard | The same tool rejects an unrelated or non-child session id |
| Follow-up result | Child session processes the follow-up and returns a non-empty assistant response |

## Validation Commands

```bash
# Focused automated coverage for this setup:
cargo test -p everruns-core agent_handoff -- --nocapture

# Core regression coverage:
cargo test -p everruns-core --lib -- --test-threads=1

# Server compile check:
cargo check -p everruns-server
```

## Failure Modes

| Failure | What to look for |
|---------|-----------------|
| Handoff starts before connection | `required_connections` is not being resolved through `UserConnectionResolver` |
| Source can use Fake AWS tools directly | Target capabilities leaked into the source runtime instead of remaining on the target Agent |
| Credential appears in events/resource metadata | Tool args, prompt context, resource metadata, or event logging is persisting secrets |
| Child session has wrong agent | `start_agent_handoff` is creating from inherited config or blueprint config instead of configured target `agent_id` |
| Handoff cannot be listed | `session_resources.kind = "agent_handoff"` registration or filtering is broken |
| Follow-up reaches wrong session | `message_agent_handoff` is missing the `parent_session_id` ownership check |
