# AgentMail email transport

Source-only adapter; not part of the published alpha. The host supplies its API
key, an exact inbox identity and a bounded HTTP transport. Credentials have no
Debug representation. All requests use the fixed AgentMail HTTPS origin.

`AgentMail::verify_inbox` verifies access before binding an inbox. `list_received`
returns at most 100 message summaries with a continuation token. `get_message`
fetches one body, validating both inbox and message identity. The caller decides
which previews warrant a full read.

`verify_received` verifies Svix HMAC against the original request bytes and a
bounded timestamp before parsing. Only `message.received` becomes incoming work;
receipts and security notifications do not launch another agent. The host must
persist and deduplicate before acknowledging, and reconcile provider history.

`EmailSend` preserves text and optional HTML. Sending requires a stable, host-owned
idempotency key. Replies set explicit recipients, disable reply-all, retain the
provider's original subject, and mark the message as an automatic reply. Sending
never retries internally. A timeout, server failure or unreadable acceptance is
`DeliveryUnknown`; a definite rate limit supplies retry advice. Hosts must persist
immutable send intents and reconcile unknown acceptance before retrying.

`recipient_tag` recognizes a base mailbox and its `+tag` recipient variants. Keep
the original recipient. A tag never authenticates a sender or grants access to a
thread; quota and provider identity remain those of the base inbox.

Attachments are metadata only. This surface does not claim chat streaming,
attachment transfer, sender authorization, template rendering, durable waiting,
or a generic chat Adapter implementation. Those decisions belong to the host.

## Verification

`cargo test -p channelkit-agentmail` runs synthetic signature and transport
contracts. The ignored `live` test requires `AGENTMAIL_API_KEY` and
`AGENTMAIL_TEST_INBOX`, sends only to a unique plus variant of that same designated
inbox, checks both body alternatives and repeats the identical idempotent send.
It fails when prerequisites are missing. Live plus delivery and duplicate-send
protection were verified on 2026-11-25. Provider-origin webhook delivery is not
established by this test.
