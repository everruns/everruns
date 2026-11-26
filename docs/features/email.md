---
title: Email inboxes
description: Connect AgentMail inboxes to an agent with batch triage, message review and text/HTML replies.
appliesTo: [platform]
---

Email is an agent channel. The same agent can also have Slack or other channels;
email conversations keep their own audience and history. This implementation
requires data plane v2 and configured system decisions for automatic processing.

## Connect an inbox

Open **Email → Providers** and save an AgentMail API key as an organization
provider account. Create the inbox in AgentMail, then add its existing address
under **Email → Inboxes**. Registration verifies access to the inbox. Provider
account saving alone verifies key format, not delivery.

Copy the webhook URL shown by Everruns into a new AgentMail `message.received`
webhook. Then paste its signing secret into Everruns and save to enable the inbox.
The inbox remains disabled until this step is complete. Saving a new signing
secret also supports key rotation. Disable the inbox to pause intake and sending.
Everruns also reconciles newly received mail through bounded polling.

In the agent's Channels settings, add **Email**, select the inbox, describe the
requests the agent should handle, and publish the channel. One inbox has one
receiving channel. Automatic execution and automatic sending are separate opt-ins;
both are off initially. Review held messages before approving execution, and
review the rendered reply before approving its send.

`support@agentmail.to` and `support+one@agentmail.to` reach the same registered
inbox. Tags do not create another account, quota or authenticated identity.

## Read progressively

Inbox lists show subjects, senders and bounded previews. Open a message for the
clean view; switch to **Full message** for preserved quoted context. Both are
untrusted input. Attachments are listed as metadata and are not downloaded or
sent in this version.

Incoming bursts enter a durable backlog. Triage examines batches of previews,
fetches selected bodies and independently checks relevance and answerability.
Uncertain messages are held. Ending an agent turn waits for the next admitted
message without running a polling loop.

## Compose and review

Templates accept named scalar substitutions such as `{{body}}` and `{{subject}}`.
HTML values are escaped and active markup removed. Provide either text or HTML;
Everruns derives the missing alternative. The preview shows both. Reply templates
must use the supplied body and subject variables. AgentMail replies keep the
original thread subject.

Each successful turn produces at most one immutable reply intent, addressed only
to the incoming From mailbox. Later template edits cannot change that intent.
An output check or an administrator authorizes sending separately from agent
execution. If delivery is uncertain, the reply remains **unknown** for operator
reconciliation; it is never blindly sent again.

## Boundaries

Email sessions permit scoped email reads and replies only. They cannot invoke
Slack, shell, files, delegation or arbitrary MCP tools. Email is not a login, and
matching an address does not link someone to a Slack identity. Use instructions
and preloaded context suitable for external correspondents. Automated model
checks reduce risk but do not guarantee prompt-injection detection.

Completed and ignored message bodies expire after 30 days; receipt identities are
kept for another 90 days. Held messages and uncertain deliveries require review
and continue to count toward storage limits.
This expiry applies to the inbox's stored bodies. Session transcripts and copies
held by the email provider follow their own retention policies.
