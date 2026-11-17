# Run an Everruns agent

The user's agents keep working in Everruns after this conversation ends, with
their own tools, sandbox and credentials. Use one when the task fits an agent
that already exists, runs longer than this conversation should wait, or needs
access only that agent has.

## 1. Pick the agent

If the user named one, find its id. Otherwise list candidates and propose one:

- `everruns_home` shows agents and recent sessions in hosts that render it.
- `everruns agents list --search <word>` (MCP `query`, or the CLI) returns ids
  and names; `everruns agents get <id>` shows its instructions and capabilities.

Confirm the choice in one line when it is not obvious. If nothing fits, offer
to build one ([build-agent.md](build-agent.md)).

## 2. Write the brief

The agent sees none of this conversation. Its first message must stand alone:

- the goal and what "done" looks like (format, length, where to put output);
- the inputs: links, ids, repository and branch, or files you will upload;
- constraints the user stated (deadline, sources to avoid, who to ask).

Write it like a ticket for a capable colleague. Do not paste secrets into it.

## 3. Start and follow it

| Step | MCP | CLI |
|---|---|---|
| Start | `agent_run { "agent_id": "agent_…", "message": "<brief>", "title": "…" }` returns `session_id` | `everruns sessions create --agent agent_… --title "…"`, then `everruns chat "<brief>" --session session_… --no-stream` |
| Check | `session_get_status { "session_id": "…", "since_event_id": "<last seen>" }` | `everruns sessions get <id>`; messages: `everruns sessions messages list --help` |
| Wait for the reply | Poll `session_get_status` until the status is `idle` | `everruns chat "<message>" --session <id>` waits by default |

Statuses: `active` is working, `idle` means the turn finished and the agent
waits for the next message, `waiting_for_tool_results` means it waits on a
client-side tool or an approval, `paused` was stopped on purpose.

Open-ended tasks: cap the spend when the user asks for a limit, with
`everruns sessions create … --budget-limit 5` (US dollars) instead of `agent_run`.

Long tasks: tell the user the session is running, give the session link
(the `ui_link` a read returns), and check back rather than
polling in a tight loop. The session keeps going without you.

## 4. Questions and approvals

Some agents ask the user a question or need approval before a sensitive tool
call. Those are the user's to answer, not yours:

- Show the question or the pending action in your reply.
- Point the user at `everruns_home` (it lists what waits on them) or the
  session chat link. Then check the status again.

If a tool reports `connection_required`, call `connect` so the user can link
the account. If the agent needs a key for this session, use
`session_set_secret`; the user types the value on the page it returns.

## 5. Files in and out

Each session has its own file system.

- **Upload inputs** (CLI): `everruns files push --session <id> ./input` copies a
  local folder into the session. Over MCP, create files with
  `everruns sessions fs create --help`.
- **Read outputs**: `everruns files ls --session <id> --recursive`, then
  `everruns files pull --session <id> ./out`. Over MCP, `everruns sessions fs
  get --help` reads a path. Link the files view: `/sessions/<id>/files`.

## 6. Bring the result back

Report what the agent produced (summarize; quote only what matters), which
files it wrote and where you saved them, and the session link. Say plainly if
it did not finish, and why.

## Follow-ups

- Same thread of work: `session_send_message` (MCP) or `everruns chat "…"
  --session <id>` keeps the agent's context.
- Try a variant without disturbing the original: `everruns sessions fork --help`.
- Stop a turn that went wrong: `everruns sessions cancel --help`; then see
  [debug-session.md](debug-session.md).
