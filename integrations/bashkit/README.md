# everruns-integrations-bashkit

> Sandboxed Bash execution for Everruns agents.

`everruns-integrations-bashkit` adapts the standalone Bashkit interpreter to
Everruns session filesystems, cancellation, progress, narration, and egress
policy. Network-capable shell builtins remain an explicit capability
configuration opt-in.

Part of the [Everruns](https://everruns.com) ecosystem. Framework applications
enable it with the `bashkit` feature; advanced hosts can register the
capability and hook dispatcher directly.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_bashkit::BashkitShellCapability;

assert_eq!(BashkitShellCapability.id(), "bashkit_shell");
```

## What It Provides

- Sandboxed `bash` tool backed by Bashkit
- Live session-filesystem and indexed-search adapters
- Cooperative cancellation, progress, and output sanitization
- Egress-routed HTTP and user-hook dispatch

## Documentation

- [Framework capability integrations](https://docs.everruns.com/framework/capability-integrations/)
- [API reference](https://docs.rs/everruns-integrations-bashkit)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
