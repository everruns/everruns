# everruns-integrations-sprites

> Persistent microVM sandboxes for Everruns agents.

`everruns-integrations-sprites` gives agents persistent, hardware-isolated Linux
microVMs through the [Sprites](https://sprites.dev) (Fly.io) REST API. Sprites are
Firecracker VMs with full ext4 filesystems that persist across sessions, so an
agent can keep long-lived state between runs.

Part of the [Everruns](https://everruns.com) ecosystem — the durable agentic
harness engine for building unstoppable agents. It registers with `everruns-core`
through the Everruns integration plugin system.

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_sprites::SpritesCapability;

let capability = SpritesCapability;

assert_eq!(capability.id(), "sprites");
```

## What It Provides

- Persistent, hardware-isolated Linux microVMs (Firecracker VMs)
- Filesystems that persist across sessions
- Command execution and file access inside the microVM
- Bring-your-own Sprites API key via the user connection provider
- Inventory-based Everruns integration registration

## Documentation

- [Sprites integration](https://docs.everruns.com/integrations/sprites/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
