# Portable agent examples

`dad-jokes.md` is an existing-style Markdown agent. `triage/` is a versioned
TOML folder with instructions, declared root-level files, a complete .agents/skills tree
and a disabled AG-UI channel description. Neither embeds platform IDs or credentials.

```bash
cargo run -p everruns-cli -- agents validate examples/agent-packages/dad-jokes.md
cargo run -p everruns-cli -- agents validate examples/agent-packages/triage
cargo run -p everruns-cli -- agents import examples/agent-packages/triage
cargo run -p everruns-cli -- agents export triage --format folder --out ./triage-export
cargo run -p everruns-cli -- agents diff examples/agent-packages/triage --against ./triage-export
```

The following executes a real Framework session offline with a deterministic
model, using either definition:

```bash
cargo run -p everruns --example file_agent -- examples/agent-packages/triage
```

To serve a folder, an application can replace macro discovery with
`serve::start(serve::App::builder().agent_package("examples/agent-packages/triage").build()).await`.
Omitted models use serve's simulator. Declare a gateway model for live inference.
File channels describe intent; configure serve channel transports separately.

Public reference: https://docs.everruns.com/how-to/define-agents-as-files/

`project-review/` shows an agent definition beside a project. It explicitly
selects `src/**`; its README is not automatically packaged.
