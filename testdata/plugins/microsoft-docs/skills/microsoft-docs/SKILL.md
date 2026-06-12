---
name: microsoft-docs
description: Search official Microsoft documentation, API references, and code samples via the Microsoft Learn MCP server. Use when a question involves Azure, .NET, Windows, Microsoft 365, or other Microsoft products.
license: MIT
---

# Microsoft Docs

Use the `microsoft-learn` MCP server tools to answer questions about
Microsoft products from official documentation instead of memory.

## Workflow

1. Search the docs with a focused query (product name plus the specific
   concept, e.g. "Azure Functions timeout limits").
2. Prefer fetching the full page for the most relevant result before
   answering; search snippets alone are often truncated.
3. Cite the learn.microsoft.com URL of every page you rely on.
4. If results conflict, prefer the page whose product version matches the
   user's context, and say which version you used.

## Notes

- The server is public and read-only; no authentication is required.
- For code samples, quote them verbatim from the docs and link the source.
