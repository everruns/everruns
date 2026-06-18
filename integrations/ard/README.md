# everruns-integrations-ard

Agentic Resource Discovery (ARD) client integration for [Everruns](https://everruns.com).

Provides the experimental `resource_discovery` capability so agents can discover
external capabilities (MCP servers, A2A agents) via operator-configured ARD
registries ([spec](https://agenticresourcediscovery.org/spec/)) and attach them
into the session config layer.

Tools:

- `discover_resources({ text, filter?, registry_id? })` — search a configured registry.
- `attach_resource({ urn, registry_id? })` — resolve, verify trust, SSRF-validate, and attach.
- `list_attached_resources()` — list attached resources.

See [`SPEC.md`](./SPEC.md) for architecture and security review, and
[`docs/integrations/ard.md`](../../docs/integrations/ard.md) for the quick start.
