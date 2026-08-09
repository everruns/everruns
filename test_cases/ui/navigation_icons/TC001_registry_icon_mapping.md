# TC001: Registry navigation icon mapping

## Description

Verify registry domains use distinct semantic icons consistently across desktop navigation, the
responsive drawer, page mastheads, and command search.

## Preconditions

- Everruns is running on the full DB-backed stack with authentication disabled.

## Test Data

| Domain | Expected icon |
|---|---|
| Skills | Book |
| Capabilities | Blocks |
| Plugins | Plug |
| MCP servers | MCP glyph |

## Steps

1. At a desktop viewport, open `/skills` and inspect the Registries section of the left navigation.
2. Confirm Skills, Capabilities, Plugins, and MCP servers use the expected distinct icons.
3. Open `/capabilities`, `/plugins`, and `/mcp-servers`; confirm each masthead matches its navigation icon.
4. Open command search and query each registry domain; confirm page and entity results use the same icon.
5. At a narrow viewport, open the navigation drawer and repeat the icon check.
6. Close and reopen the drawer; confirm icon sizing, alignment, active state, accessible names, and drawer behavior are unchanged.

## Expected Result

- Skills uses the book icon, Capabilities uses blocks, Plugins uses a plug, and MCP servers retains
  its custom MCP glyph everywhere the registry domains are represented.
- All four entries remain visually distinct at desktop and narrow widths.
- Navigation sizing, alignment, active states, accessible labels, and responsive behavior are unchanged.
