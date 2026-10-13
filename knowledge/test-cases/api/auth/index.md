# Auth (API)

* [TC001: Auth - Credential and Tenant Boundaries](TC001_tenant_and_credential_boundaries.md) - Verify explicit organization selection, header precedence, tenant isolation, and non-member resource resolution using disposable authenticated fixtures.
* [TC002: Auth - Scope and Permission Parity](TC002_scope_and_permission_parity.md) - Verify read-only credential restrictions and caller permissions agree across REST, command dispatch, and MCP, with no side effects on denial.
* [TC003: Auth - Refresh Rotation and Replay](TC003_refresh_rotation_and_replay.md) - Verify single-use refresh rotation, bounded concurrent consumption, logout revocation, and isolation between independent browser sessions.
* [TC004: Auth - Deployment Modes](TC004_auth_modes.md) - Verify none, admin, full, and external deployments expose the intended sign-in paths and reject unsupported authentication configuration.
