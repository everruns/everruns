# everruns-integrations-lua

> Sandboxed Lua and code-mode execution for Everruns agents.

`everruns-integrations-lua` provides the high-risk, opt-in `lua` interpreter
capability and its `lua_code_mode` routing companion. Scripts run in vendored
Lua 5.4 with bounded memory, instructions, time, output, filesystem access, and
host-provided tool/HTTP bridges.

Part of the [Everruns](https://everruns.com) ecosystem. Enable it through the
Framework `lua` feature or register it directly in an advanced host.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_lua::LuaCapability;

assert_eq!(LuaCapability.id(), "lua");
```

## What It Provides

- Sandboxed Lua 5.4 execution
- Session-filesystem and controlled tool-call bridges
- Optional host egress bridge with network policy
- Code-mode tool-definition routing

## Documentation

- [Framework capability integrations](https://docs.everruns.com/framework/capability-integrations/)
- [API reference](https://docs.rs/everruns-integrations-lua)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
