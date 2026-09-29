# OpenAI Agents API runtime prototype

This standalone prototype maps an Everruns `RuntimeAgent`-shaped JSON document to
the preview OpenAI Agents API. It demonstrates one client-side function tool, one
remote HTTP MCP server, function-result submission, and canonical Everruns event
projection. It does not register a production worker backend.

The path is off by default. Set `EVERRUNS_OPENAI_AGENTS_API=1` and
`OPENAI_API_KEY` only for an explicit experiment. The API key must have
`api.agents.read`, `api.agents.write`, and `api.responses.write`.

Run the offline protocol and event-mapping tests:

```sh
cd examples/openai-agents-api
python3 -m unittest -v
```

The tests make no network requests and need no credentials. They cover request
mapping, MCP configuration, required function actions and results, streamed text,
terminal failures, unknown events, incomplete streams, and reconnect recovery.

For a live check, create `agent.json`:

```json
{
  "model": "gpt-6-astra",
  "system_prompt": "Use get_customer before answering.",
  "tools": [{
    "type": "client_side",
    "name": "get_customer",
    "description": "Look up a customer",
    "parameters": {
      "type": "object",
      "properties": {"id": {"type": "string"}},
      "required": ["id"],
      "additionalProperties": false
    }
  }]
}
```

Create `mcp.json`:

```json
{
  "server_label": "openai_docs",
  "server_url": "https://developers.openai.com/mcp",
  "tool_name": "search_openai_docs"
}
```

The CLI deliberately implements only the example `get_customer` function. It is
not a generic tool host. An unconfigured function fails instead of being reported
as successful. Run:

```sh
EVERRUNS_OPENAI_AGENTS_API=1 OPENAI_API_KEY="$OPENAI_API_KEY" \
  python3 prototype.py --agent agent.json --mcp mcp.json \
  "Look up customer 123 and check the Agents API documentation."
```

The architecture record at
[`knowledge/execution/openai-agents-api-prototype.md`](../../knowledge/execution/openai-agents-api-prototype.md)
defines the gaps and adoption bar.
