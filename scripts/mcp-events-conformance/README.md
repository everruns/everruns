# MCP conformance scope

The base job uses released `@modelcontextprotocol/conformance@0.1.16`, with a
committed npm lockfile, to check `server-initialize` through Everruns' `/mcp`.
It is a baseline smoke check, not a claim that the full core suite passes.

The Events job uses unmodified upstream revision
`c8625de3118c2d5dc0561696cddeb0baaf124f2c` from stacked client PR
[541](https://github.com/modelcontextprotocol/conformance/pull/541), including
server PR [521](https://github.com/modelcontextprotocol/conformance/pull/521).
Neither stable 0.1.16 nor published 0.2.0-alpha.12 lists Events scenarios. The
source revision above contains all nine; its package version alone does not
identify that code. Install with
`bash scripts/mcp-events-conformance.sh --setup-only`; omit the flag to run.

Four of the nine optional scenarios run: server discovery and webhook delivery,
and client webhook refresh and no-extension fallback. The client driver uses the
production trigger API and refresher. The outbound fixture uses a synthetic PAT,
production `/mcp`, URL validation, signing and delivery, with only the final egress
hop mapped to the official loopback receiver. TLS/network egress is not assessed.
A receiver hook binds its listener to loopback and records its port; upstream
scenario logic is unchanged. Delayed 503 responses make fresh retry signatures
observable. No application credentials or provider calls are used.

Every raw upstream check is retained. Discovery cannot exercise the core
`subscriptions/listen` prerequisite or its two poll-based unknown-name probes.
The delivery harness cannot change DNS between subscribe and delivery. These four
exact upstream failure/untestable diagnostics are recorded as unverified; neither
full server scenario is reported as passing. Changed diagnostics and all protocol
failures fail the job, and named required checks must be present and successful.
Warnings and skipped checks also remain unverified in the summary. Domain tests
separately cover unknown-name errors and production delivery URL validation;
they do not certify the unexercised upstream probes.

Additional Everruns checks use the same real `/mcp` fixture and a signed loopback
receiver to verify independent message IDs across subscriptions and callback
consent reuse across refreshes and arguments. They are recorded separately in
`everruns-checks.json`; they are not upstream scenarios or official assertion
counts. The identity check demonstrably fails against the preceding server even
when its 36 supported upstream assertions pass.

Poll and push scenarios (four total) are not run: Everruns offers webhook delivery.
The separate server subscription-management scenario hardcodes an unresolvable
`conformance.invalid` callback, incompatible with production callback verification;
it is not run or claimed. Domain tests cover subscription management, tenant/access
checks, signed termination, ingress signatures and transport deduplication.
