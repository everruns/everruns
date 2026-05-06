# TC007: Cross-Org Memory Store Isolation

## Description

Verify that memory stores and memories are strictly scoped to the active organization: switching orgs hides the other org's stores, and direct access to a foreign store ID returns a non-disclosing 404 (not 403).

## Preconditions

- Server running with full mode (`just start-all`) and authentication enabled
- User belongs to two organizations: **Org A** and **Org B**
- Org A has at least one store (e.g. `team-knowledge`); note its ID `mst_<A>`
- Org B has at least one store with a different name; note its ID `mst_<B>`

## Test Data

| Field | Value |
|-------|-------|
| Org A store ID | `mst_<A>` (from Org A) |
| Org B store ID | `mst_<B>` (from Org B) |

## Steps

1. Sign in and switch the active organization to **Org A** via the org switcher
2. Navigate to **Memory** (`/memory-stores`)
3. Capture the list of stores shown
4. Switch the active organization to **Org B**
5. Navigate to **Memory**
6. Capture the list of stores shown
7. While in Org B, hit `GET /v1/memory-stores/<mst_A>` (Org A's store ID) using the user's session, with `X-Org-Id: <org-b-public-id>`
8. While in Org B, hit `GET /v1/memory-stores/<mst_A>/memories` with the same headers
9. While in Org B, hit `DELETE /v1/memory-stores/<mst_A>/memories/<mem_id_from_A>`

## Expected Result

- Step 3: Only Org A's stores are listed; `mst_<B>` is not present
- Step 6: Only Org B's stores are listed; `mst_<A>` is not present
- Step 7: Returns `404 Not Found` with a generic not-found error body — never `403 Forbidden` and never any field that confirms the store exists in another org
- Step 8: Returns `404 Not Found` with the same non-disclosing shape
- Step 9: Returns `404 Not Found`. The memory remains active when re-checked from Org A (active count unchanged)
- The capability-config store picker, when shown for an agent in Org B, never lists Org A's stores even if the agent JSON is hand-edited to reference `mst_<A>`; instead the picker displays "Selected store is no longer available in this organization."
