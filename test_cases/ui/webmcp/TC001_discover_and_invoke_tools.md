# TC001: Discover and invoke WebMCP tools

## Description

Verifies that the authenticated Everruns UI registers its browser-native shell tools and that a
browser agent can invoke read and navigation tools.

## Preconditions

- Development stack running in auth mode `none`.
- Effective `webmcp` feature flag enabled for the default organization.
- Chromium with WebMCP enabled, or the repository's browser smoke-test ModelContext harness.

## Test Data

| Field | Value |
|---|---|
| Navigation target | Sessions page |

## Steps

1. Open `/chats` — the landing surface — and wait for the application to settle.
2. Discover registered tools from `document.modelContext`.
3. Verify `everruns_get_context`, `everruns_search`, and `everruns_open` are present.
4. Execute `everruns_get_context` and verify it reports the current org and the Chats page.
5. Execute `everruns_open` for the Sessions page.
6. Verify the visible URL changes to `/sessions`.

## Expected Result

- Shell tools are discoverable and return bounded structured results.
- `everruns_open` visibly navigates to the Sessions page.
- No registration or execution error affects ordinary UI operation.

## Smoke-test paths

The hermetic CI path runs `apps/ui/e2e/webmcp.spec.ts`. It installs a ModelContext harness with
Chrome's asynchronous `getTools()` and `executeTool(tool, jsonString)` contract, mocks only the HTTP
API, and exercises the rendered application.

For a native local smoke, run the development stack with `FEATURE_WEBMCP=true`, opt the test
organization into `webmcp`, and launch Chrome 149+ through `agent-browser` with
`--enable-blink-features=WebMCPTesting`. Use `agent-browser eval` to await
`document.modelContext.getTools()`, select a tool descriptor by name, and pass it to
`document.modelContext.executeTool()`. This path must not install the harness.
