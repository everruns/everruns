# everruns-builtins

Portable policy capabilities for the Everruns Framework.

This crate owns the backend-neutral implementations that shape agent behavior,
including context compaction, tool search, budgeting, loop and progress guards,
prompt caching, tool-call repair, output handling, and guardrails. Linking the
crate does not register anything: applications choose a registry and call
`register_portable_capabilities` explicitly.

```rust
use everruns_builtins::register_portable_capabilities;
use everruns_core::CapabilityRegistry;

let mut registry = CapabilityRegistry::new();
register_portable_capabilities(&mut registry)?;

# Ok::<(), everruns_capability::CapabilityError>(())
```

The bundle contains policy, not environment integrations. It does not own a
network client, process runner, interpreter, database, server, or hosted
service. Capabilities that persist or distill tool output declare a dependency
on `session_file_system`; the embedding application must compose a compatible
filesystem implementation when enabling those capabilities.

Use the `everruns` facade for the normal Framework API. Depend on this crate
directly when building a custom capability registry or a minimal host.
