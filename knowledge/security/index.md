# Security, auth, and governance

* [Authentication Specification](authentication.md) - Authentication modes and OAuth.
* [Personal Access Token Scopes](pat-scopes.md) - Design for enforcing PAT scopes (`*`, `read`); enforced on REST and `/v1/commands`.
* [Encryption Specification](encryption.md) - Envelope encryption for sensitive data.
* [Bound Secrets](bound-secrets.md) - Write-only API keys the platform puts on requests to one URL; the agent sees a placeholder.
* [Audit Logging](audit-logging.md) - Audit logging.
* [Threat Model](threat-model.md) - Security threat model.
* [Security Testing](security-testing.md) - Security testing process (threat-model tests, fail-rs, DeepSec, supply chain).
* [Multitenancy](multitenancy.md) - Organization-based multitenancy.
* [Assignment-scoped Journal Read Foundation](worker-assignment-read-scope.md) - Preparatory server scope and deny-by-default RPC gate; production identity integration remains blocked.
* [Permissions Model](permissions.md) - Fine-grained permissions model.
* [Scoped Resource Ownership and Access](resource-access.md) - Proposed shared ownership, grants, and runtime authorization for connections, credential bindings, and GitHub; memory deferred.
* [Scoped Resources Implementation Request](scoped-resources-implementation-request.md) - Six-stage implementation handoff for common access, credential capture, and GitHub compatibility; memory excluded.
* [Feature Flags](feature-flags.md) - Feature flags system.
* [Budgeting Specification](budgeting.md) - Extensible budgeting system.
* [Usage Tracking Specification](usage-tracking.md) - LLM token usage tracking.
* [Machine Payments](machine-payments.md) - Capability-side payments to external paid services.
* [Secret-leak Guardrails](secret-leak-guardrails.md) - Defense-in-depth semantic detection and deterministic known-value redaction.
* [CI Secret Handling](ci-secret-handling.md) - Keeping runtime-fetched credentials out of Actions logs.
