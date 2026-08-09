# everruns-platform

Backend platform identity entities for the [Everruns](https://everruns.com)
agentic runtime.

This crate owns the durable backend identity aggregates that were historically
defined in `everruns-core`:

- `Organization`
- `Principal`

It also owns the auth-facing identity values that no core code names
(`OrgMembership`, the `ANONYMOUS_USER_*` seed constants, the org public-id
generation/validation helpers, and the `PrincipalStatus` lifecycle enum).

Cross-cutting value types that core's runtime and permissions layer embed
(`OrgRole`, `PrincipalKind`, `PrincipalSummary`) and the `DEFAULT_ORG_*`
multitenancy constants remain in `everruns-core` and are re-exported here for a
unified consumer surface.

The dependency direction is strictly `platform -> core`; `everruns-core` never
depends on `everruns-platform`.

## License

MIT
