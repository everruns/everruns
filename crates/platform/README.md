# everruns-platform

Backend platform identity entities for the [Everruns](https://everruns.com)
agentic runtime.

This crate owns the durable backend identity aggregates that were historically
defined in `everruns-core`:

- `Organization`
- `Principal`

Cross-cutting identity value types that core's runtime and permissions layer
embed (`OrgRole`, `OrgMembership`, `PrincipalKind`, `PrincipalStatus`,
`PrincipalSummary`) and the multitenancy constants remain in `everruns-core` and
are re-exported here for a unified consumer surface.

The dependency direction is strictly `platform -> core`; `everruns-core` never
depends on `everruns-platform`.

## License

MIT
