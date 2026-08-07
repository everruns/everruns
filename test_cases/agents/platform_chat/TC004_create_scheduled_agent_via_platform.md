# TC004: Create a Scheduled Agent via Platform Chat

## Description

Verify that Platform Chat can discover models and control-plane operations,
create an Agent with an explicit default model, and create an Agent Trigger for
recurring autonomous work. This is the regression case for the failed
dad-joke/Visti provisioning turn.

Automated behavioral coverage: `evals/platform-capability`, preset
`provisioning`. That study uses a documented dummy API key and verifies the
created Agent, model, encrypted MCP registration and attachment, Agent Trigger,
and absence of a Platform Chat Session Schedule through public resource APIs.

Credential entry is intentionally outside this test. Use an MCP server that
requires no secret, or preconfigure its Agent-scoped connection. A secret stored
in Platform Chat's session must never be treated as available to the new Agent.
Credential binding itself is covered by the automated dummy-credential case;
never paste a production credential into this manual test transcript.

## Preconditions

- Control plane running with a real tool-calling model for Platform Chat
- Signed-in user is authorized to assign the requested Agent capabilities
- At least one active model exists
- A reusable no-secret MCP server is available, or omit MCP from the prompt

## Steps

1. Open Platform Chat at `/chat`.

2. Send:

   ```text
   Create an agent named hourly-joke that tells a short dad joke. Use the active
   model whose model ID is gpt-5.6-terra. Run it hourly using an Agent Trigger.
   Inspect existing resources first, reuse them where possible, and show me the
   final agent and trigger links.
   ```

3. Platform Chat must ask for confirmation before creating the reusable
   organization-wide Agent. Confirm the exact Agent, model, and trigger request.

4. Inspect the tool trace after confirmation. Verify that Platform Chat:
   - uses `discover` to find model, Agent, and Agent Trigger operations rather
     than calling an invented tool such as `read_models`;
   - uses `query` to resolve `gpt-5.6-terra` and inspect existing resources;
   - uses `execute` for the requested creates;
   - queries the final Agent and Trigger state before answering.

5. Open the returned Agent link and verify its `default_model_id` points to the
   model whose provider model ID is `gpt-5.6-terra`.

6. Open the returned Trigger link and verify it targets the new Agent and uses
   an hourly schedule.

7. Verify no Session Schedule was added to the Platform Chat session.

## Expected Result

- The Agent and Agent Trigger exist and are linked from the final answer.
- The Agent has the requested default model.
- The trigger, not Platform Chat, owns the recurring autonomous execution.
- No `read_models`, `prepare_agent_provisioning`, or other invented planning
  tool appears.
- Tool calls render as structured blocks and the final answer begins with the
  outcome, without internal reasoning or narration.

## Negative paths

- If `gpt-5.6-terra` is absent or inactive, Platform Chat reports that fact and
  does not silently choose another model.
- If the caller cannot assign a requested high-risk capability, `execute`
  returns a permission error and no elevated Agent is created.
- Supplying `organization_id` to any Platform tool is rejected.
- A partial multi-command failure is followed by `query`; already-created
  resources are reused or reported instead of blindly duplicated.
