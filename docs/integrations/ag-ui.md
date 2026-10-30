---
title: AG-UI support
description: AG-UI 1.0 streaming, tools, interrupts and state support in Everruns, and how the event protocol relates to the chat component gallery.
appliesTo: [framework, platform, cloud]
---

Everruns supports **AG-UI 1.0**, the event protocol used by CopilotKit and
other agent frontends. **A2UI** is the component format rendered inside chat
messages. Use the [component gallery](/reference/generative-ui/) to browse
all 69 supported components, or [Generative UI in chat](/features/generative-ui/)
to enable them in console chat and Work Chat.

## Protocol support

| Feature | Support |
| --- | --- |
| Run lifecycle and streaming assistant text | Supported |
| Reasoning and tool activity | Controlled by the endpoint's visibility policy |
| Client-provided frontend tools | Supported; return trailing tool messages to resume parked calls |
| Questions and resume | Supported through interrupts |
| Tool approval interrupts | Available when explicitly enabled on the channel |
| Token usage and subagent events | Available when explicitly enabled on the channel |
| Shared state | Todo snapshots and deltas when enabled; client-provided state is not applied |
| Capability discovery | Available on the channel's capabilities endpoint |

For a configured AG-UI channel, post `RunAgentInput` to
`POST /v1/channels/{channel_id}/ag-ui`. Discover the channel's declared features
at `GET /v1/channels/{channel_id}/ag-ui/capabilities`, using the same authentication
as a run. The [channel guide](/features/channels/#ag-ui) covers channel setup and
client migration. To host AG-UI from a Rust application, see
[Serve AG-UI](/framework/ag-ui/).

Public Chat uses the same projection with a restricted policy: approval
interrupts, usage, subagent events and shared state are disabled. An anonymous
AG-UI channel can explicitly enable those features; choose its disclosure policy
before exposing it publicly.

## Chat component support

Console chat and Work Chat share the A2UI 1.0 standard catalog, including bound
forms, charts, tables, layouts, media and composite cards. Form submissions send
visible messages through the conversation's existing permissions and approval
path. Busy, archived and read-only conversations disable message actions.

The native chat hosts render inline surfaces. They do not execute
`callRendererFunction`, `callAgentFunction`, or OpenUI Query/Mutation calls.
Maps and media depend on their sources being reachable; generated HTML runs in
a sandbox without scripts or external network access. The
[generative UI guide](/features/generative-ui/) describes compatibility with
saved nested A2UI and OpenUI messages.
