# TC005: Ground Plugin and Connection State Before Agent Creation

## Objective

Verify that Platform Chat distinguishes operation discovery from resource
inspection and grounds an Agent-creation confirmation in authoritative plugin,
capability, Agent, connector, and current-user connection reads.

## Preconditions

- A DB-backed stack with `AUTH_MODE=none` and a real tool-calling model.
- The `resend` plugin is installed and active.
- One active Agent references the installed plugin's exact `plugin:plugin_…`
  capability ref.
- Run both connection variants: no current-user Resend OAuth connection, then
  a connected Resend OAuth account.

## Steps

1. Open Platform Chat — `/chats` → **New chat** → built-in **Platform Chat** harness — and send: `Create another agent with resend plugin.`
2. Before confirming, inspect the tool trace.
3. Verify Platform Chat uses `query` to read the installed plugins,
   capabilities, Agents, connection providers, and current-user connections.
   The reads may be combined into one bounded script. Discovery, if needed,
   identifies operations only and is not repeated for the same concept.
4. Verify the response identifies the installed Resend plugin by name/link,
   reports it active/available and already attached to one Agent, and separately
   reports the current user's OAuth connection state.
5. Verify Platform Chat asks for confirmation before creating the reusable
   org-wide Agent. In the disconnected variant, the confirmation explains that
   the user must connect Resend before the Agent can use its OAuth-backed tools.
6. Repeat with two installed resources whose names both match a shared search
   term. Verify Platform Chat presents the candidates and asks which one to use;
   it does not guess.
7. Disable the target plugin and repeat. Verify Platform Chat reports the
   disabled state and does not propose attaching it as available.
8. Use a unique absent name and repeat. Verify absence is concluded only after
   authoritative resource reads, never from a zero-match `discover` result.

## Expected

- Installed, active/available, attached, and connected are independent facts.
- Connected and disconnected current-user states are accurate and secret-free.
- Disabled, absent, and ambiguous-name states are not collapsed together.
- No operation-catalog miss is presented as evidence that a resource is absent.
- Preflight is short and deterministic, preserves org/user boundaries, and
  shows names and useful UI links instead of raw IDs in prose.
- Agent creation remains behind explicit confirmation.
