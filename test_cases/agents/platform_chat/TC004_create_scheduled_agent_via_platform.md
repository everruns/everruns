# TC004: Create a Scheduled Agent via Platform Chat

## Description

Verify that Platform Chat can discover models and control-plane operations,
create an Agent with an explicit default model, and create an Agent Trigger for
recurring autonomous work. This is the regression case for the failed
dad-joke/Visti provisioning turn.

Automated behavioral coverage verifies the encrypted Agent credential binding
and runtime-only MCP parameter injection with a disposable sentinel. Never paste
a production credential into this manual test transcript.

## Preconditions

- Control plane running with a real tool-calling model for Platform Chat
- Signed-in user is authorized to assign the requested Agent capabilities
- At least one active model exists
- A controlled Visti-compatible MCP test endpoint is available. It exposes
  `visti_send` with a required `channel_key` and records the received request
  without sending an external notification.

## Steps

1. Open Platform Chat: go to `/chats`, start a **New chat**, and pick the built-in **Platform Chat** harness.

2. Send:

   ```text
   Create an agent named hourly-joke that sends a short dad joke through the
   attached Visti-compatible MCP server. It requires a channel key. Use the
   active model whose model ID is gpt-5.6-terra and run it hourly using an Agent
   Trigger. Show me how to configure the credential securely.
   ```

3. Platform Chat must ask for confirmation before creating the reusable
   organization-wide Agent. Confirm the exact Agent, model, and trigger request.

4. Inspect the tool trace after confirmation. Verify that Platform Chat:
   - uses `discover` to find model, Agent, and Agent Trigger operations rather
     than calling an invented tool such as `read_models`;
   - discovers exact command names and follows their `bash_usage`; it does not
     probe builtins with `--help`, shell inventory, or guessed flags such as
     `--model_id` on `create_agent`;
   - uses `query` to resolve `gpt-5.6-terra` and inspect existing resources;
   - uses `execute` for the requested creates and chains dependent returned IDs
     with `jq` rather than manually converting resource identifiers;
   - queries the final Agent and Trigger state before answering.
   - does not call unrelated Generic tools such as Bash, web fetch, session
     secrets, or Session Schedule tools.

5. Open the returned Agent link and verify its `default_model_id` points to the
   model whose provider model ID is `gpt-5.6-terra`.

6. Open the returned Trigger link and verify it targets the new Agent and uses
   an hourly schedule.

7. Verify no Session Schedule was added to the Platform Chat session.

8. Verify Platform Chat did not ask for a key. Open its returned secure setup
   link, enter a disposable test key, and save. The value must remain masked and
   must not appear in the page after the request completes.

9. Invoke the Agent once against the controlled endpoint. Verify the endpoint
   received the bound `channel_key`, while the Session messages, tool-call
   details, SSE events, server/worker logs, browser state, and screenshots do
   not contain the disposable value.

## Expected Result

- The Agent and Agent Trigger exist and are linked from the final answer.
- The Agent has the requested default model.
- The trigger, not Platform Chat, owns the recurring autonomous execution.
- No `read_models`, `prepare_agent_provisioning`, or other invented planning
  tool appears.
- Tool calls render as structured blocks and the final answer begins with the
  outcome, without internal reasoning or narration.
- The credential works for the Agent Trigger's session mode without being
  copied into each Session.

## Negative paths

- If `gpt-5.6-terra` is absent or inactive, Platform Chat reports that fact and
  does not silently choose another model.
- If the caller cannot assign a requested high-risk capability, `execute`
  returns a permission error and no elevated Agent is created.
- Supplying `organization_id` to any Platform tool is rejected.
- A partial multi-command failure is followed by `query`; already-created
  resources are reused or reported instead of blindly duplicated.
