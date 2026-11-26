# Consumer verification and secrets

## Test Channelkit integration boundaries

For deterministic provider tests, inject an implementation of
`channelkit::http::Transport` with the adapter's `.transport(...)` method. Use synthetic
credentials and controlled responses, or a bounded loopback HTTP server. Do not rely
on upstream private test-support crates, repository fixtures, or example packages being
available as dependencies. Keep request headers containing credentials out of recordings.

Exercise the behavior the integration owns:

- Send raw signed fixtures through the host's actual webhook route, verify normalized
  events and replies, and reject tampered bodies or mismatched installation scope.
- Check handler selection/subscription behavior and host authorization for reactions,
  buttons, and stop controls. Verify host acceptance succeeds before acknowledgement.
- Cover provider rejection, rate limits, unknown/partial send acceptance, and cancellation
  where used. Assert that host retry policy does not duplicate unconfirmed sends.
- Test restart/persistence behavior in the host when required; Channelkit's bounded memory
  state does not supply durable recovery. Assert unsupported provider operations honestly.

## Live verification belongs to the host

Use the consuming application's existing secret manager, environment injection, and
provider account configuration. See [provider variables](rust.md#provider-configuration).
No particular secret-manager vendor, project name, local account, or configuration is
required by Channelkit. Never print tokens/signing secrets or commit credential files.

Run live operations only with authorization for designated test channels/recipients.
Make a missing prerequisite an explicit failure for an invoked live test; an ordinary
offline test run need not require live credentials. Bound test count, duration, media
size, and cost, and report exactly which operations ran.

For Slack, verify the intended workspace, bot membership, and required scopes. Test the
host's supported post/readback/edit, reactions, and media paths; remove only artifacts
created by the test. Missing scopes are a setup failure, not proof of an adapter defect.

For WhatsApp, require an explicitly opted-in test recipient. Template tests need an
approved template and language; free-form replies need valid reply-window context.
Accepted template sends do not prove delivery or reading. Messages cannot be cleaned up
through the adapter, so use synthetic content and a dedicated recipient.

Provider-origin E2E requires the host's reachable webhook and actual provider-generated
messages/interactions/status events. Synthetic signed fixtures prove local verification
paths; API acceptance and remote readback prove different operations. Keep those results
separate rather than describing a limited smoke as complete provider E2E or a library release.
