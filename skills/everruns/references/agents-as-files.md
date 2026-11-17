# Agents as files

An agent package is a folder the CLI can validate, compare, import and
export. The same package works on the Platform, in the Framework and in serve.
This needs the `everruns` CLI (see [cli.md](cli.md) for setup).

## Layout

```text
agents/triage/
├── agent.toml          # the manifest (or agent.md / agent.yaml / agent.json)
├── instructions.md     # used when the manifest has no inline instructions
├── runbook.md          # a file the agent can read, selected in `files`
└── .agents/
    └── skills/
        └── investigate/
            └── SKILL.md
```

```toml
schema_version = 1
name = "triage"
display_name = "Triage assistant"
capabilities = ["session_file_system", "current_time"]
max_iterations = 20
files = ["runbook.md"]

[mcpServers.issues]
use = "catalog:linear"      # an MCP server already configured in the organization
actsAs = "user"

[channels.chat]
type = "ag_ui"              # channels are created disabled unless `enabled = true`
```

Rules worth knowing:

- Only files listed in `files` (paths or globs) are shipped to the agent.
- Skills under `.agents/skills/<name>/SKILL.md` are picked up, and the `skills`
  capability is added for them. A skill's `name` must match its folder.
- Omit the model and harness to use the organization's defaults.
- Never put secrets in the folder. Reference configured MCP servers with
  `use = "catalog:…"`, or `${VAR}` placeholders the destination resolves.

Full format: <https://everruns.com/docs/how-to/define-agents-as-files/>.

## Commands

```bash
everruns agents validate ./agents/triage            # offline, no sign-in
everruns agents validate ./agents/triage --remote   # also checks models, capabilities, MCP servers
everruns agents diff ./agents/triage --target triage   # what import would change
everruns agents import ./agents/triage               # create
everruns agents import ./agents/triage --target triage --reason "…"   # update by name
everruns agents export triage --format folder --out ./agents/triage
```

`diff` and `validate` never change anything. `import` replaces the agent's
configuration and upserts declared channels; channels the folder omits are
left alone, and existing enablement and credentials stay as they are.

## Typical flows

- **Start from an agent built in the UI**: `export --format folder`, commit it.
- **Change via pull request**: edit the folder, run `validate` and
  `diff --target`, include the diff in the PR, `import` after merge.
- **CI check**: run `everruns agents validate <dir>` on every pull request; it
  needs no credentials. Add `--remote` with an `EVERRUNS_API_KEY` repository
  secret to also check references against the organization.
- **Copy to another org or deployment**: `export` from one, then `import` with
  `--api-url` or another `EVERRUNS_API_KEY` for the other.
