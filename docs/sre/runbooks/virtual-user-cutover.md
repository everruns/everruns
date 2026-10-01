---
title: Virtual User Cutover
description: Upgrade runtime accounts and connections without duplicating credentials across organizations.
---

This upgrade replaces agent identities and the global user connection store with organization-scoped virtual users. Existing identity IDs, session IDs, conversation history, archive state, and pins are preserved. Everruns login accounts, roles, organization membership, and personal access tokens remain management identities.

Stop old API and worker processes before running this upgrade. The schema rename and internal protocol change require all processes to use the new version together; mixed-version rolling deployment is unsupported. Drain active runs before stopping workers. Existing historical messages are retained, while new execution records capture the current speaker and responder. Restart the upgraded API to apply migrations, then start upgraded workers and UI.

Accounts with one organization have their encrypted provider grants moved automatically. Accounts with several organizations retain grants in a pending cutover queue that execution cannot read. Each account owner chooses a single destination in Settings → Connections. The move preserves the grant ID and encrypted payload, and rekeys resource cleanup to that virtual user. An existing destination grant is never overwritten. Resources awaiting an ambiguous grant move cannot be cleaned up until a grant is available on their organization-scoped owner. Moving a grant unblocks resources in the chosen organization. Resources in other organizations require explicit reconnection there; credentials are never copied between organizations.

Legacy session-local OAuth grants have no verified caller binding and require reconnection. Agents continue using their existing service accounts. External identities retain qualified workspace bindings where available; unqualified historical actors require verified ingress before becoming a new runtime subject.

After restart, check health, open an existing chat, and verify its pin and archive state. Compare provider grants in Settings → Connections and the default virtual user's Connections tab. Test a current user turn, a service turn, and an unattended user attachment: the last must request setup rather than borrow a human owner's grant. Confirm the pending queue is empty only after users have selected destinations. Keep backups and the existing encryption keys throughout the upgrade.
