# MCP research

An agent.toml folder with a remote, unauthenticated HTTP MCP dependency and a
separate disabled chat channel. No model or harness is pinned: the destination
uses its defaults. Framework applications must enable MCP and bind their model.

```bash
everruns agents validate examples/agents/mcp-research
everruns agents validate examples/agents/mcp-research --remote
everruns agents import examples/agents/mcp-research
```

Offline validation checks the declaration; it does not contact the server.
A live session contacts [DeepWiki's public MCP server](https://mcp.deepwiki.com/).
Choose an enabled destination model and ask: “Explain how owner/repository handles
configuration.” Replace owner/repository with a real public repository.
The channel must be published separately before external ingress is accepted.
