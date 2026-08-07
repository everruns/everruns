# TC001: Auto-Switch Organisation from Direct Resource Link

## Description

Verify that following a direct link to a top-level resource (session, agent,
app, harness, eval, agent identity) owned by an organisation the user is a
member of but has **not** currently selected causes the UI to transparently
switch to the owning org and render the resource — instead of showing a
"not found" screen.

Covers the cross-org resource resolution behaviour described in
`knowledge/security/multitenancy.md` (Cross-Org Resource Resolution). The API's
`GET /v1/<resource>/<id>` continues to return 404 for cross-org access; the
recovery happens only in the UI via the authenticated
`GET /v1/resolve-org` endpoint, which only reveals orgs the caller already
belongs to.

## Preconditions

- User is authenticated.
- User is a member of at least two organisations — call them **Org A** and
  **Org B**.
- Org A has at least one visible session. Capture its ID (for example
  `session_019db85695a8785e87e8203109109343`) and its URL, e.g.
  `/sessions/session_019db85695a8785e87e8203109109343/chat`.
- The browser's currently selected org (the `everruns_org` cookie + sidebar
  switcher) is **Org B** — NOT the owner of the captured session.

## Test Data

| Field                  | Value                                                      |
|------------------------|------------------------------------------------------------|
| Owning org             | Org A                                                      |
| Currently selected org | Org B                                                      |
| Direct link            | `/sessions/<session_id_owned_by_A>/chat`                   |
| Expected result        | Page renders the session; sidebar shows Org A as selected. |

## Steps

1. Confirm the sidebar org switcher shows **Org B** as current.
2. Navigate the browser directly to the captured URL
   `/sessions/<session_id_owned_by_A>/chat` (e.g. paste into the address bar
   and press Enter, or click an external link that points there).
3. Wait for the page to settle (loading skeleton disappears).

## Expected Result

- The sidebar org switcher now shows **Org A** as the current organisation.
- The session detail page renders normally — chat view, title, metadata are
  all visible.
- The URL in the address bar is unchanged (still the original
  `/sessions/<id>/chat` — no redirect to `/sessions`).
- No "Session not found" error is displayed.
- Opening DevTools → Network, the sequence of requests shows (order may vary
  slightly):
  1. `GET /api/v1/sessions/<id>` → **404** (still Org B context).
  2. `GET /api/v1/resolve-org?id=<session_id>` → **200** with
     `{ "org_id": "<Org A public_id>", "org_name": "<Org A name>" }`.
  3. `POST /api/v1/users/me/switch-org` with Org A's public id → **200**.
  4. `GET /api/v1/sessions/<id>` → **200** (now Org A context).

## Negative Variants (should each keep the existing 404 behaviour)

The following variants verify the enumeration guarantee is preserved:

1. **Unknown session ID** — navigate to
   `/sessions/session_00000000000000000000000000000000/chat`. Expected: page
   shows "Session not found" and `GET /api/v1/resolve-org?id=...` returns
   **404**. No org switch occurs.
2. **Session owned by a non-member org** — log in as a user who is NOT a
   member of the session's owning org and open its direct link. Expected:
   page shows "Session not found" and `GET /api/v1/resolve-org?id=...`
   returns **404**. No org switch occurs.
3. **Logged-out user** — sign out, then open the direct link. Expected: the
   user is redirected to login (standard auth flow). No call to
   `/v1/resolve-org`.

## Cross-Resource Coverage

Repeat step 2 with each of the following direct URL shapes to confirm the
fallback works for every top-level entity with a detail route. Each case
should switch to the owning org and render the detail view — the fallback is
installed once in the shared CRUD detail hook (`useDetail`) and in
`useSession` / `useAgentIdentity`, so all of the below should behave
identically.

- `/agents/<agent_id>`
- `/agent-identities/<identity_id>`
- `/apps/<app_id>`
- `/evals/<eval_id>`
- `/harnesses/<harness_id>`
- `/sessions/<session_id>/chat`
