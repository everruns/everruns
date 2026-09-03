---
title: URL mode elicitation
description: When an MCP server needs a secret, an authorization, or a payment, Everruns holds the turn and asks a person to finish it in their browser — the value never passes through the client or the model.
sidebar:
  label: URL elicitation
---

Some tool calls cannot be completed by an agent alone. A billing server needs the
customer to authorize a charge with their bank; an analytics server needs the
user's own API key; a provider needs an OAuth consent screen clicked. The value
involved must never reach the agent: not the MCP client, not the model's context,
not the event log.

MCP's answer is **URL mode elicitation** (protocol `2026-07-28`). Instead of
asking the client for the value, the server answers `tools/call` with a URL and
waits. Everruns supports it on both sides.

## As an MCP client: the turn holds

When a tool call comes back with a URL elicitation, Everruns pauses the turn and
puts the URL in front of the person, with the domain highlighted:

<video
  controls
  playsinline
  preload="none"
  width="1024"
  poster="/videos/mcp-url-elicitation-consent.jpg"
  src="/videos/mcp-url-elicitation-consent.mp4">
</video>

The user asks for a charge, the server needs their bank's authorization, the turn
holds on a consent card, and the tool runs once they come back and confirm.

Consent is collected in **two steps** — *Open link*, then *I've finished —
continue* — because only the person knows when the interaction on the other side
actually finished. Answering the server the moment the tab opens resumes the turn
too early, and the server simply asks again.

### Entering a secret

The same flow carries values the agent must never see. Here the server needs the
user's own API key and collects it on its own page:

<video
  controls
  playsinline
  preload="none"
  width="1024"
  poster="/videos/mcp-url-elicitation-enter-a-secret.jpg"
  src="/videos/mcp-url-elicitation-enter-a-secret.mp4">
</video>

The key goes from the user's browser straight to the provider. The Everruns
transcript carries the report, never the key.

### What the client guarantees

- **The capability is declared only when a human can answer.** A host with no way
  to reach a person declares no `elicitation` capability at all, so a compliant
  server cannot ask.
- **The URL is validated before anyone sees it**: `https` only (loopback `http`
  for local development), so a consent surface is never handed a `javascript:` or
  `file:` URL. The client never fetches it.
- **The domain is shown, and Punycode is flagged.** Internationalized domains are
  legitimate but can impersonate; the card says so.
- **Consent is single use and bound to one domain.** A server that elicits
  `pay.example.com`, waits for the click, then elicits somewhere else on the retry
  gets no reuse of that consent — the user is asked again.
- **A refusal is final.** Declining ends the call and tells the agent to continue
  without the tool.

## As an MCP server: Everruns serves the form

Everruns' own `/mcp` endpoint uses the same mechanism when a client asks it to
store a secret or connect a provider. `session_set_secret` never accepts a value
as a parameter: it answers with a URL to a form Everruns serves, the user types
the value there, and the retry confirms it is stored. The MCP client that started
the call only ever holds the URL.

The page requires the visitor's own session on top of the signed link, and
refuses anyone but the user the elicitation was minted for — the link alone
grants nothing, which is what closes the phishing case the spec warns about.

## Clients that cannot render a card

Pausing is a client capability, declared per session:

```json
{ "hints": { "url_elicitation": true } }
```

The Chat UI declares it automatically. A client that does not gets the older
behaviour: the turn continues and the elicitation reaches the user through the
tool result, as an actionable `url_elicitation_required` payload with the URL and
the server's reason.

To complete such a call from your own client, see
[Complete a URL elicitation over the API](/how-to/complete-a-url-elicitation/).

## Protocol support

URL mode elicitation is `2026-07-28` only. In earlier eras elicitation is a
server-initiated request over a server-to-client stream this transport does not
open, so Everruns declares nothing regardless of what the host can do.

## Related

- [MCP](/features/mcp/), Everruns on both sides of the protocol
- [Capabilities](/features/capabilities/), how MCP servers become agent tools
