# everruns-http

> Concrete HTTP transports for Everruns hosts.

`everruns-http` provides the Reqwest-backed `EgressService` implementation while
keeping concrete runtime-network dependencies out of `everruns-core` and the
default offline Framework build.

Part of the [Everruns](https://everruns.com) ecosystem. Hosted server and
worker composition install these transports explicitly; offline embedders use
the disabled egress contract from core.

## Quick Example

```rust
use everruns_http::DirectEgressService;

let transport = DirectEgressService::new();
# let _ = transport;
```

## What It Provides

- Reqwest-backed egress with allowlist and DNS-pinning enforcement
- Bounded streaming responses and redirect-safe policy handoff
- Explicit host-owned transport selection

## Documentation

- [Network access](https://docs.everruns.com/advanced/network-access/)
- [API reference](https://docs.rs/everruns-http)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
