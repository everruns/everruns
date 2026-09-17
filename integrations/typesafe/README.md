# everruns-integrations-typesafe

> Typed judgments from [TypeSafe](https://typesafe.ai) for Everruns agents.

[![Crates.io](https://img.shields.io/crates/v/everruns-integrations-typesafe.svg)](https://crates.io/crates/everruns-integrations-typesafe)
[![Documentation](https://docs.rs/everruns-integrations-typesafe/badge.svg)](https://docs.rs/everruns-integrations-typesafe)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/everruns/everruns/blob/main/LICENSE)

`everruns-integrations-typesafe` contributes the `typesafe` capability: one
tool, `typesafe_evaluate`, that lets an agent ask typed questions about content
and get calibrated numbers back — a probability, a selected option, a graded
level — instead of forming a second impression in prose. Use it to verify,
rate, route, or classify.

Part of the [Everruns](https://everruns.com) ecosystem, the durable agentic
harness engine for building unstoppable agents. It registers with
[`everruns-core`](https://crates.io/crates/everruns-core) through the Everruns
integration plugin system.

Looking for the client on its own, with no Everruns dependency? That is
[`typesafe-systemone`](https://crates.io/crates/typesafe-systemone).

## Quick Example

```rust
use everruns_core::capabilities::Capability;
use everruns_integrations_typesafe::TypeSafeCapability;

assert_eq!(TypeSafeCapability.id(), "typesafe");
```

An agent's call looks like this:

```json
{
  "state": "Why did the chicken cross the road? To get to the other side.",
  "questions": [
    {"id": "is_funny", "type": "noul", "instructions": "Would a general audience laugh?"},
    {"id": "humor", "type": "score", "instructions": "How funny is it?",
     "levels": ["Not funny at all", "Mildly amusing", "Genuinely funny", "Hilarious"]}
  ]
}
```

## What It Provides

- The `typesafe` capability and its `typesafe_evaluate` tool, with answers
  rendered decision-ready (a score also carries `normalized`, `level`, `label`)
- A user-scoped TypeSafe API-key connection, with a `TYPESAFE_API_KEY` session
  secret as fallback
- `TypeSafe::new(key)` for `AgentBuilder::capability`, keeping an
  application-owned credential inside the client
- Inventory-based Everruns integration and connector registration

## Documentation

- [API reference (docs.rs)](https://docs.rs/everruns-integrations-typesafe)
- [Design notes](SPEC.md)
- [TypeSafe integration](https://docs.everruns.com/integrations/typesafe/)
- [Everruns documentation](https://docs.everruns.com)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
