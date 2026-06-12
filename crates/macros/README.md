# everruns-macros

> Procedural macros for the Everruns workspace.

`everruns-macros` provides the proc macros used across the Everruns crates, such
as the `#[audit]` attribute that emits audit-event logging around service
methods. (`#[audit]` records audit events; it does not gate authorization —
command-layer authorization is enforced uniformly by `Command::run`.) It is an
internal building block of the Everruns workspace, used by `everruns-core` and
the server.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents.

## What It Provides

- The `#[audit]` attribute macro for audit-event logging on service methods
- Compile-time helpers shared by Everruns crates

## Documentation

- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
