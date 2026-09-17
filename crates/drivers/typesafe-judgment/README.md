# everruns-typesafe-judgment

Deployment-owned judgment service: backs core's provider-neutral
`JudgmentService` with the [TypeSafe System One](../typesafe) client.

The credential is `UTILITY_TYPESAFE_API_KEY`, owned by the deployment and never
agent- or session-configurable. The agent-facing capability is a separate
surface in `integrations/typesafe`, with its own user-scoped connection.

Not published while `typesafe-systemone` is private.

```rust,ignore
use everruns_typesafe_judgment::SystemJudgmentConfig;

let judgment_service = SystemJudgmentConfig::from_env().into_service();
```
