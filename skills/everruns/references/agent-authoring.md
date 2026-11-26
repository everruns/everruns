# Author an Everruns agent

Use the connected MCP server's `discover`, `query`, and `execute` tools.
Their `everruns` commands execute on the server; do not require a local CLI or
send REST requests with the MCP OAuth token. Select the user's organization and
pass `organization_id` consistently.

## Establish the behavior and dependencies

Translate the user's request into instructions, permitted actions, expected
outputs, and conditions that require human input. Resolve only missing details
that materially affect behavior. For an existing agent, read its current
definition and manager context before editing; preserve unrelated configuration.

Inspect available harnesses, capabilities, models, skills, and MCP catalog
entries through resources or discovered list/get commands. A harness supplies
the base environment; an agent overlays behavior and configuration. Reuse a
suitable harness and add capabilities only for required actions. Conversation
is plain chat with no files; an agent that researches, keeps notes, runs on a
schedule, or holds one long session belongs on Worker, which already includes
files, saved tool output, compaction and web search. Do not re-add a capability
the harness provides: an empty copy replaces the harness's settings with
defaults. Preview the agent and resolve its findings before saving. Choose a model
available in the target organization and preserve the user's explicit choice.

## Start from an example

Review the examples before writing a new definition. They are ready-made
agents and harnesses that ship with the deployment but are not in the
organization until someone adopts them, so `agents list` and `harnesses list`
do not show them. A harness missing from `harnesses list` may well be an
example: Coding, the Yolop-based harness for software work on repositories,
and Data Analyst are examples, not built-ins. Do not fall back to Sandbox
Worker or Worker for coding work before checking.

```bash
everruns agents examples list       # MCP: list_agent_examples
everruns harnesses examples list    # MCP: list_harness_examples
everruns harnesses examples adopt coding --reason 'Coding agents need the Coding harness'
everruns agents examples adopt pr-reviewer --reason 'Set up PR reviews for the repo'
```

Adopting creates an ordinary org agent or harness named after the example (a
short suffix is added when the name is taken); check `harnesses list` first so
you do not adopt the same harness twice. An adopted harness is then selected
by name like any other. An agent example already pairs a prompt with the
capabilities it needs; adapt it rather than copying parts of it by hand. A
template's guided setup (GitHub App, connections, trigger) is not run by
adoption: do those steps next.

MCP catalog entries are presets, not automatically active tools. Discover the
agent MCP attachment commands before adding a server. Select `actsAs` according
to whose account should perform the work: `user` requires the chatting person's
connection, `service` uses the agent's identity, and `user_or_service` explicitly
permits fallback. Unattended schedules require a usable service identity. Let
the user authorize connections through Everruns rather than collecting tokens.

## Apply a focused definition

Discover the exact deployed command and use its returned usage and input schema.
For a simple new agent, `everruns agents create` accepts `--name` and
`--system-prompt` in the server command tree; follow discovered usage if the
deployed version differs. For a portable definition, `everruns agents import --content` supports
inline content through MCP; discover its format and target options first.
Check for an existing name before creating or importing to avoid an unintended
upsert.

Portable packages keep one `agent.toml` definition, initial files, and complete
`.agents/skills/<name>/` trees. Supporting references and scripts belong with
their SKILL.md. Use the package import/export and validation operations offered
by the server rather than creating a parallel representation. Never embed
credentials in a definition or skill. Discover command support for validation,
diff, and preview; some file-oriented CLI commands require local files and
cannot be used directly by an MCP-only client.

For an update, carry the manager-context revision you read and a concrete
`--reason` describing the requested change. Read back the stored definition and,
where available, preview its effective harness and capability composition.
Configuration saved successfully is not evidence that the behavior works.

## Memory

Conversation and Worker include the **Memory** capability (`agent_memory`):
stores under `/memory` the agent writes with `remember` and searches with
`recall`. Configure it only to change a default: each store is a switch
(`off`, `read`, `read_write`) and named memories are attached by name. For a
coordinator that starts threads or spawned agents, keep `team` read-write so
the workers share its decisions and can resolve blocked items. For agents that
work on the same area without a coordinator, create a named memory
(`everruns memories create --name <name>`) and attach it to each one:

```json
{"ref": "agent_memory", "config": {"attach": [{"memory": "everruns-maintenance", "mode": "read_write"}]}}
```

Tell the agent in its instructions which decisions to record and where, such as
"record a person's decision on a PR in team memory with their words as the
quote". Never store a person's private context in team, org or named memory.
Read what it remembered with `everruns memories stores` and
`everruns memories entries <memory_id>`.

## Deploying to a channel

A channel (Slack, a webhook, public chat, an API key) is how people reach the
agent; discover the channel commands and connect one only when asked. For
Slack, start from the `slack-team-assistant` example. Keep the **Reactions**
capability: it makes the agent react 👀 when it starts on a Slack message and
✅ once it has answered, so people see it got the message. Connecting the
agent's first Slack channel turns it on. For an agent whose Communication is
explicit, Everruns adds the reactions itself. Change how the agent reacts in its
instructions ("Don't react to messages", "Use 🫡 instead of 👀") only when the
person asks; do not restate the default. Add the **Slack** capability when the
agent should also act in the workspace: update its own messages, share files,
or look people up.

### Scheduled coordinator in a Slack channel

For an agent that does recurring work and reports in one Slack channel (for
example a housekeeping agent), adopt `slack-channel-coordinator`. It talks
explicitly, so its notes stay out of the channel and Everruns reacts 👀/✅ on
messages it works on. Fill in its "Your job" section and owner list, then:

1. **Draft the channel:** `everruns agents channels create --agent-id <agent>
   --channel-type slack --channel-config '{"channel_id":"C0123456789"}'`. The
   `channel_id` binds it to one Slack channel; leave it out to answer anywhere
   the bot is invited. A new channel is a draft and answers nobody.
2. **Install:** the person selects **Add to Slack** on the channel page (it
   needs their browser and Slack consent; give them the channel's `ui_link`).
   A successful install publishes the channel when they may publish.
3. **Publish** only if it is still a draft (`status` in `agents channels get`):
   `everruns agents channels publish --agent-id <agent> --channel-id <appchan>`.
4. **Invite:** in Slack, `/invite` the app into the channel. Private channels
   always need this; Health reports "Slack bot is not in its channel" until it
   is done.
5. **Point triggers at the channel:** create the schedule with
   `--target-channel-id <appchan> --target-binding-key slack:channel:<C…>` so
   runs post into the channel conversation (`slack:channel:` matches the
   default per-channel session strategy). A second schedule whose message asks
   for the daily report closes the day. The result shows the target under
   `.config.target`.
6. **Check:** run one schedule now (`everruns agents triggers trigger`) and
   read the channel, not only the session.

## Verify the behavior

When authorized to run the agent, use `agent_run` with a representative request,
then inspect the session output and events. Verify the requested result, the
tools used, and whether a connection, secret, question, or approval blocked
progress. Use an available analyze/health-check operation when its cost and side
effects fit the task; discover its contract first. Do not claim a live test
when you only validated or previewed the definition.

Test side-effecting behavior within the user's authorized scope. A request to
create a scheduled agent does not imply permission to perform its external
action immediately. Configure schedule/channel publication only when requested,
using discovered commands, and report whether it is saved, verified, connected,
and active.

If interrupted after a write, query the stored state before retrying. Summarize
the final behavior and unresolved requirements, with the returned agent or
session link when available.

## Execution choices

Use [execution-options.md](execution-options.md) when selecting native tools,
Bashkit, Monty Python Code Mode or a full session sandbox. Bashkit already
provides Python; choose Monty for its Python tool interface and durable
program continuations, not for language availability.
