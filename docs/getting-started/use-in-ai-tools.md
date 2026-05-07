---
title: Use in AI Tools
description: Set up Everruns in AI tools through the Everruns(Dev) plugin.
---

Use Everruns from the AI tools where you already work. The Everruns(Dev) plugin
ships in this repository with both Claude Code and Codex support.

## Claude Code

The Claude Code plugin connects Claude Code to `https://dev.everruns.com/mcp`
and adds Everruns(Dev) tools, slash commands, and a skill with platform
guidance.

### Install the Plugin

1. Add the Everruns marketplace and install the plugin from inside Claude Code:

   ```text
   /plugin marketplace add everruns/everruns
   /plugin install everruns-dev@everruns-dev
   ```

   Claude Code reads `.claude-plugin/marketplace.json` from the repository to
   discover available plugins. The install syntax is
   `<plugin-name>@<marketplace-name>`.

2. Verify the install by running:

   ```text
   /everruns-dev:whoami
   ```

   On first run the plugin opens an OAuth flow in your browser to authenticate
   against the Everruns(Dev) platform over MCP.

If `/plugin` is not recognized, update Claude Code to a version that supports
the plugin marketplace and try again.

### Use Everruns

Ask Claude Code for an Everruns task in natural language, for example:

```text
Create an Everruns(Dev) agent that summarizes https://news.ycombinator.com/ and run it once.
```

The plugin also exposes slash commands such as `/everruns-dev:agent-run`,
`/everruns-dev:session-send`, and `/everruns-dev:discover` for common
workflows.

### Alternative Install

To install from a local clone or point at a self-hosted Everruns deployment,
see [`plugins/everruns-dev/README.md`](https://github.com/everruns/everruns/blob/main/plugins/everruns-dev/README.md).
To target another Everruns deployment, update the `url` in
`plugins/everruns-dev/.mcp.json` to that deployment's `/mcp` endpoint.

## Codex

The Codex plugin connects Codex to `https://dev.everruns.com/mcp` and adds
Everruns(Dev) tools and guidance.

### Set Up the Marketplace

1. Add the Everruns plugin marketplace:

   ```bash
   codex plugin marketplace add https://github.com/everruns/everruns.git
   ```

2. Restart Codex if **Everruns(Dev)** does not appear in the plugin directory.

3. Open the Codex plugin directory, choose the **Everruns(Dev)** marketplace
   source, and install **Everruns(Dev)**.

   ![Everruns Dev plugin page in Codex showing the Add to Codex button](./codex-everruns-dev-plugin.png)

   Codex discovers the marketplace from `.agents/plugins/marketplace.json`. That
   marketplace points to `./plugins/everruns-dev`, which contains the Codex plugin
   manifest and MCP server configuration.

4. Complete the browser OAuth flow when Codex asks you to authenticate.

For the general Codex marketplace format, see the
[Codex plugin marketplace documentation](https://developers.openai.com/codex/plugins/build#how-codex-uses-marketplaces).

### Use Everruns

Ask Codex for an Everruns task in natural language, for example:

```text
Create an Everruns(Dev) agent that summarizes https://news.ycombinator.com/ and run it once.
```

To verify the connection, ask Codex:

```text
Check my Everruns(Dev) user and active organization.
```

To point the plugin at another Everruns deployment, update the `url` in
`plugins/everruns-dev/.mcp.json` to that deployment's `/mcp` endpoint.
