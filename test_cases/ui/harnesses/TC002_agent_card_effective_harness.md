# TC002: Agent Card Effective Harness

## Description

Verify agent cards show the harness a newly created session will use and distinguish explicit
selection from organization-default inheritance.

## Preconditions

- DB-backed stack is running with authentication disabled for local testing
- The organization has active Base and Generic harnesses
- One agent pins Generic, one agent inherits the organization default, and one agent has an
  unavailable harness reference

## Test Data

| Agent | Effective harness | Source | Expected card state |
|-------|-------------------|--------|---------------------|
| Explicit agent | Generic | Explicit | Linked `Generic` value with `Explicit` badge |
| Inherited agent | Organization default | Organization default | Linked effective name with `Org default` badge |
| Unavailable agent | Unresolvable | Explicit | Honest unavailable text without a link |

## Steps

1. Open `/agents` at desktop width in grid view.
2. Verify each card's harness row, source badge, tooltip, and link behavior against the table.
3. Select list view and repeat the checks.
4. Set a narrow mobile viewport and repeat the grid and list checks.
5. Create a session from the inherited agent without a harness override and verify its harness ID
   matches the effective harness shown on the card.

## Expected Result

- Every agent card includes a compact Harness detail row.
- The tooltip names the effective harness and its source, including `organization default` for
  inherited agents.
- Long harness names truncate without overflowing and remain available through accessible labels.
- Active and archived resolved harnesses link to their detail page; deleted or unresolved values do
  not present a misleading link.
- Capability badges, status, edit action, IDs, loading/empty states, and responsive layout remain
  usable.
