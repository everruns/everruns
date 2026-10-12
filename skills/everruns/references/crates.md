# Dependencies for Framework applications and advanced hosts

Start with `everruns`. Select only the features the application needs; use the
installed version's Cargo features and documentation as the authority.

| Dependency | When needed |
|---|---|
| `everruns` | Application agents, Engine-owned sessions, tools, providers, and workspace policies |
| `everruns-core` with `host` | Advanced composition, backend replacement, and host phase contracts via `everruns_core::host` |
| `everruns-core` with `engine` | Backend-neutral execution kernel via `everruns_core::engine` |
| `everruns-contracts` | Neutral service, provider, capability, and execution contracts |
| `everruns-drivers` | Direct vendor-driver composition for an advanced host |
| `everruns-integrations` | Direct integration composition for an advanced host |

Do not add retired `everruns-host`, `everruns-engine`, or `everruns-mcp` crates.
Their current boundaries are modules/features of `everruns-core`.

## Select facade features

Enable a provider such as `openai` and configure `OpenAI::from_env()` for a live
model. `Model::simulated` bundles an offline provider for deterministic tests.
Provider driver identities are wire IDs, not a closed enum of all providers.

Use `mcp` for scoped remote MCP and `mcp-stdio` for opt-in local processes.
Integration features such as `filesystem`, `bashkit`, `web-fetch`, `duckduckgo`,
and `lua` select their concrete implementations. Host command execution through
`host-shell` or `host-compute` requires a deliberate containment policy.

The `local` feature supplies local persistence and workspace support. When
replacing stores directly, use `HostBackends` from `everruns_core::host` and
keep the canonical event log as the conversation write path. Connection-aware
tools require the host's credential resolver; a capability alone supplies no
user credentials.

Use `everruns::batteries::runtime_builder()` when an advanced host wants the
facade's selected integration presets. Supplying a custom `HostComposition`
replaces the default capability registry; register each capability that the
host will reference.

- [Custom backends](https://everruns.com/docs/framework/custom-backends/)
- [Capability integrations](https://everruns.com/docs/framework/capability-integrations/)
- [Capability boundaries](https://everruns.com/docs/framework/capability-boundaries/)
- [Workspaces and environments](https://everruns.com/docs/framework/workspaces-and-environments/)
- [Core API](https://docs.rs/everruns-core)
- [Contracts API](https://docs.rs/everruns-contracts)
