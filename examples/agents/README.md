# File-based agent examples

Start with a TOML folder. All definitions use stable names, omit resource IDs and
credentials, and keep ordinary files relative to the directory root. Instructions
live beside the manifest. [Format reference](https://docs.everruns.com/reference/agent-package/)
and [how-to guide](https://docs.everruns.com/how-to/define-agents-as-files/).

| Folder | Demonstrates | Try |
| --- | --- | --- |
| [dad-jokes-agent](dad-jokes-agent) | Minimal instructions + one capability | “Tell me a short dad joke.” |
| [triage](triage) | Root files, CSV fixture, complete skill tree, disabled chat channel | “Investigate an issue using the runbook.” |
| [project-review](project-review) | Manifest beside a project; `src/**` is selected, README stays out | “Review src/sample.py.” |
| [mcp-research](mcp-research) | Real anonymous HTTP MCP server; host model defaults; channel intent | “Explain configuration in owner/repository.” |
| [everruns-support-agent](everruns-support-agent) | Everruns troubleshooting instructions | “Help diagnose a failed session.” |
| [coding-review-agent](coding-review-agent) | Repository review, file access and Bashkit | “Review this change for material defects.” |
| [research-agent](research-agent) | Research workflow with a plan and evidence | “Research an engineering decision.” |
| [incident-commander-agent](incident-commander-agent) | Incident coordination workflow | “Triage the current incident.” |

No provider model is pinned: choose the organization's enabled model, or bind a
model in Framework code. Bashkit and file tools still depend on host policy.
These packages configure behavior; they do not include Framework code-defined
tools or production service integrations.

## Validate, import, compare and export

```bash
# Offline: parse the folder and inspect all selected assets.
everruns agents validate examples/agents/triage
# Destination checks: model, harness, capabilities, MCP and channels.
everruns agents validate examples/agents/triage --remote
# Create an agent; use --target triage for an explicit update.
everruns agents import examples/agents/triage
# Review a normalized diff without applying anything.
everruns agents diff examples/agents/triage --target triage
# Export a complete folder or ZIP for the UI import dialog.
everruns agents export triage --format folder --out ./triage-export
everruns agents export triage --format zip --out ./triage.zip
```

Upload `triage.zip` through **Agents → Import**. Preview instructions, defaults,
files, skills, MCP servers and channels; select an existing destination to review
changes. The zip starts at `agent.toml`, rather than enclosing it in another folder.

Each `agent.toml` includes the public schema directive for editor completion.
Schema validation checks the document; CLI validation also checks assets and
skill trees. Relative file selection excludes each example's README unless listed.

## Framework and serve

Run a complete Framework session with the deterministic offline model:

```bash
cargo run -p everruns --example file_agent -- examples/agents/triage
```

Applications load these folders with `AgentPackage::load(path)`. Bind executable
custom tools and MCP dependencies in host code. Serve applications can use
`serve::App::builder().agent_package(path).build()`. See the how-to for full code.

Code-first Framework walkthroughs remain in [the root examples directory](../README.md).
They have their own Cargo packages, live provider profiles, fixtures and tests.

## Legacy Markdown compatibility

[legacy/dad-jokes.md](legacy/dad-jokes.md) is the one Markdown compatibility example:

```bash
everruns agents validate examples/agents/legacy/dad-jokes.md
```

The old root Markdown examples were migrated into TOML folders, preserving
instructions and capability requirements.

## Test-only demonstration fixtures

Six older definitions depend on `fake_*` capabilities now owned by
[`everruns-test-support`](../../crates/test-support/src/capabilities/mod.rs).
They are kept in [fixtures/](fixtures/) for custom test hosts that explicitly
register those capabilities. Production Platform/worker registries deliberately
exclude them, so they are not standard imports and `just upload-agents` skips
this directory. They do not connect to real business services.

| Fixture | Demonstrates |
| --- | --- |
| [business-operations-analyst](fixtures/business-operations-analyst) | Cross-system demo analysis |
| [cloud-infrastructure-manager](fixtures/cloud-infrastructure-manager) | Cloud audit with tool hooks |
| [customer-support-agent](fixtures/customer-support-agent) | CRM support workflow |
| [devops-engineer](fixtures/devops-engineer) | Cloud operations workflow |
| [financial-analyst](fixtures/financial-analyst) | Finance records |
| [warehouse-operations-manager](fixtures/warehouse-operations-manager) | Warehouse operations |
