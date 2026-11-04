---
title: Generative UI in chat
description: Render interactive forms, charts, tables, media and cards with the same component catalog in console chat and Work Chat.
appliesTo: [platform, cloud]
---

Enable the **A2UI** capability on an Agent to let it answer with interactive UI.
Console chat and [Work Chat](/features/channels/#work-chat) render the same
69-component standard catalog: layouts, text, inputs, tables, charts, maps,
media, overlays and composite cards.

Browse the [component gallery](/reference/generative-ui/) to try live controls and copy complete A2UI examples covering all 69 components. Start with [forms and inputs](/reference/generative-ui/inputs/), [charts and tables](/reference/generative-ui/data/), or [layouts](/reference/generative-ui/layout/).

For example, ask: **“Show a release review with a chart and a form for my
feedback.”** The Agent can mix prose and widgets in one answer. Inputs and tabs
work in the browser. Submitting a form sends a visible chat message containing
its resolved context, through the conversation's existing permissions and
approval policies.

## A2UI 1.0 messages

Agents emit newline-delimited operations inside an `a2ui` fence. A minimal
surface looks like this:

````text
```a2ui
{"version":"v1.0","createSurface":{"surfaceId":"review","catalogId":"copilotkit/standard@1","components":[{"id":"root","component":"Button","label":"Review release","onClick":{"event":{"name":"review","userMessage":"Review the release"}}}]}}
```
````

The root component has ID `root`; component props are flat. Further
`updateComponents` and `updateDataModel` operations in the same answer can
update the surface, and `deleteSurface` removes it. Bind an input with
`{"@path":"/reviewer"}` and reference that path in a form's event context to
send its current value. The catalog also supports browser computations and
validation checks. User edits remain protected when an Agent updates a surface.

The [CopilotKit standard catalog](https://docs.copilotkit.ai/generative-ui/components/standard-catalog)
documents the component names and props. Everruns supplies the catalog and
writing guide through the Agent's A2UI capability; no CopilotKit agent runtime
is required.

## Saved messages and limits

Existing nested A2UI trees and OpenUI replies remain renderable. OpenUI message
actions and forms also use the conversation's message API. Enable **OpenUI**
instead of A2UI when an Agent should generate OpenUI Lang. Prefer one capability
per Agent to avoid sending two UI catalogs to the model.

Busy, archived and read-only conversations disable message actions. HTML widgets
run in a sandbox without scripts or external network access. Maps and media
depend on the source being available; maps retain a list fallback. Malformed
widgets show a fallback instead of breaking the conversation.

Chat surfaces belong to their reply. The chat hosts do not advertise external
page targets or execute `callRendererFunction`, `callAgentFunction`, or OpenUI
Query/Mutation calls against backend tools. AG-UI protocol support is separate
from this browser component catalog.
