---
type: Specification
title: Published Agent Views
description: Author configuration, successful-turn publication, and authorized rendering in Platform Chat and Work Chat.
tags: [ui, agents, channels, generative-ui]
---

# Published Agent Views

An organization opts in through the Adoption-grade `agent_views` flag. **Views**
appears in the Agent editor's **More** group beside Sandbox. The author chooses
A2UI 1.0 or static HTML, declares the data keys, and controls runtime layout
replacement and personal layouts. The **Published views** capability supplies
`publish_view`. The Agent's resolved instructions describe the publication contract.

The tool accepts a complete data object and an optional replacement layout.
A2UI binds the declared keys at the root of the data model. HTML binds `{{key}}`
in text nodes. An opaque sandbox and restrictive CSP contain the HTML renderer.
The last staged result of a successful turn becomes the snapshot if it matches
the definition revision and policy.

The control plane derives Agent, session, and turn identity from the event
stream. An Agent Trigger updates the Agent-wide snapshot. Other turns update
only their conversation. Retries and out-of-order completion preserve the
latest successful snapshot. Data planes forward the required events for
organizations with configured views through the control-plane event tap.

Full-screen Platform Chat resolves the selected conversation's Agent. Agent-wide
fallback and explicit Agent references require Agent read authority. A Work Chat
publisher can use the view alongside chat or as the channel landing page. That
page reads the Agent snapshot without starting a conversation. Bootstrap exposes
placement. The view route verifies the channel, consumer, and session before
reading content. The chat panels group a short thread list with the view and
offer separate sections and search when the list exceeds four entries.

An allowed viewer can preview, save, and reset a personal layout. The server
scopes it to their identity and audience. Both layout versions use the same
authorized data. The current definition revision gates publications and
preferences. Flag revocation blocks API reads and new publications.

## Sources

- [Agent settings](../../crates/server/src/domains/agents/views.rs)
- [Publication and personalization](../../crates/server/src/domains/agents/view_publication.rs)
- [Work Chat view endpoint](../../crates/server/src/api/agent_api/work_chat/views.rs)
- [Shared renderer](../../apps/ui/generative-ui/src/index.tsx)
