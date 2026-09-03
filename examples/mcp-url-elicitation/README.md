# URL mode elicitation

Some MCP tool calls cannot be finished by an agent alone: a charge to authorize,
an API key to paste, a consent screen to click. Under MCP `2026-07-28` the
server answers `tools/call` with a URL instead of asking for the value, and the
client puts that URL in front of a person. The value goes from their browser to
the server directly — never through the MCP client, and never into the model's
context.

This example is both halves of that:

| File | What it is |
|---|---|
| `elicit-server.mjs` | A stub MCP server that refuses to run its tool until a human enters an API key on its own page. No dependencies. |
| `consent-flow.sh` | The Everruns side over the REST API: declare the hint, read the pause, post the decision, watch the retry succeed. |

For the concepts and what the UI does with this, see
[URL mode elicitation](https://everruns.com/features/mcp-url-elicitation/). For
the API contract, see
[Complete a URL elicitation over the API](https://everruns.com/how-to/complete-a-url-elicitation/).

## Run it

The stub must be reachable **from the Everruns worker**, at an address that
passes its SSRF checks: a public origin, not `localhost` and not a private
range. Everruns blocks those deliberately, so a stub on your laptop needs a
tunnel in front of it.

```bash
# 1. Start the stub and expose it (any tunnel will do)
node examples/mcp-url-elicitation/elicit-server.mjs &
cloudflared tunnel --url http://localhost:8391      # → https://something.trycloudflare.com

# 2. Point the stub's own links at that address and restart it
PUBLIC_URL=https://something.trycloudflare.com node examples/mcp-url-elicitation/elicit-server.mjs &

# 3. Drive the flow
MCP_URL=https://something.trycloudflare.com \
  ./examples/mcp-url-elicitation/consent-flow.sh
```

The script prints the URL the server asked for. Open it, type any key into the
form, come back, press Enter — the tool then runs and the agent answers with the
report.

## What to look for

- **`GET /state` on the stub** shows the key arrived there (masked), and that the
  call only completed once it had both the key *and* an `accept` from the client.
- **The session's events** carry the report but never the key. The last step of
  the script greps for it and should find nothing.
- **Consent is single use and bound to one domain.** If the stub elicits a
  different host on the retry, the consent is not reused — a fresh
  `confirm_url_elicitation` event arrives instead.
- **Answer when the person is done, not when they click.** The server checks
  whether the out-of-band interaction actually completed; consenting at open time
  just makes it elicit again. The pause is swept after
  `TOOL_RESULT_TIMEOUT_SECS` (default 300s).
