# Ship an Everruns agent

Commands below are the `everruns` command language (see the [cli.md](cli.md)).
Creating and publishing are changes: use the CLI or MCP `execute`, pass
`--reason`, and confirm with the user before publishing anything public.

## Channel or trigger?

- **Channel**: someone outside sends a message and expects a reply.
- **Trigger**: something happens (a time, a webhook, a GitHub event) and the
  agent works without anyone waiting for a reply.

## Channels

| Type | `--channel-type` | For |
|---|---|---|
| Slack | `slack` | A Slack bot. The organization must have a Slack workspace connected |
| AG-UI | `ag_ui` | Your own web or app UI speaking the AG-UI protocol |
| Public Chat | `public_chat` | A hosted chat page for one agent |
| A2A | `a2a` | Other agents calling this one over the A2A protocol |
| FCP | `fcp` | Any HTTP client, text in and text out |

```bash
everruns agents channels create --help
everruns agents channels create --agent-id <agent_id> --channel-type public_chat \
  --reason "User wants a public chat page for the support agent"
everruns agents channels publish --help
```

A channel starts as a draft and serves traffic only when it is published, the
agent is active, and the agent's exposures are not suspended. Publishing needs
the owner-level agent permission. Slack has extra steps best done in the UI:
point the user to `<their Everruns URL>/agents/<agent_id>` > **Integrations**
(the deployment they are connected to, such as `http://localhost:9300`).

**Slack connection.** Pick how Slack reaches Everruns before creating the
channel:

- **Socket Mode** when the install has no public HTTPS address (a laptop,
  a private network). Everruns connects out to Slack. The user generates an
  app-level token in Slack (**Basic Information** > **App-Level Tokens**,
  scope `connections:write`) and pastes it into the channel's **App-level
  token** field in Everruns. **Add to Slack** needs a public HTTPS redirect,
  so without one they use **Create Slack app** under **Configure manually** and
  paste the bot token too.
- **Request URL** when Everruns has a public HTTPS `PUBLIC_APP_URL`: publish,
  then **Add to Slack**.

Tokens go into Everruns pages, never into the chat.

Voice conversations and other channel kinds are set up the same way from the
agent's **Integrations** tab; `--help` lists the types the command accepts.

Guides: <https://everruns.com/docs/features/channels/>,
<https://everruns.com/docs/how-to/publish-to-slack/>,
<https://everruns.com/docs/capabilities/slack/#socket-mode-or-request-url>.

## Triggers

```bash
everruns agents triggers create --help
everruns agents triggers create --agent-id <agent_id> --trigger-type schedule \
  --cron-expression '0 8 * * 1-5' --timezone Europe/Kyiv \
  --message "Write today's digest" --reason "Weekday morning digest"
everruns agents triggers trigger --help     # fire one now to test it
everruns agents triggers runs list --help   # see what each run did
```

Webhook, GitHub and MCP event triggers take their own flags (`--github-events`,
`--repositories`, `--mcp-server`, `--mcp-event`); read `--help` for the shape.
Guide: <https://everruns.com/docs/features/agent-triggers/>.

## Before going public

Public channels and frequent triggers spend money on every message. Offer a
budget on the agent first (`everruns budgets create --help`; the subject is the
agent, the limit in US dollars) and confirm the amount with the user.

## After shipping

Test it the way a real caller would (send a Slack message, open the chat page,
fire the trigger once), read the resulting session, and give the user the
channel or trigger link plus how to turn it off:
`everruns agents channels unpublish --help` for one channel,
`everruns agents triggers update --help` to disable a trigger, or the kill
switch for everything at once, `everruns agents pause --help`
(`resume` undoes it).
