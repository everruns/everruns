# Sidebar Prefetch Policy

## Description

Verify that authenticated app startup does not automatically prefetch visible sidebar destinations, while intent and click navigation remain responsive and Settings does not fan out to child routes.

## Preconditions

- The app is running with authentication enabled.
- A user is signed in and belongs to an organization.
- The browser network log is empty before each measured step.
- Use the same browser, viewport, environment, data, and readiness signal for before/after measurements.

## Test Data

| Input | Value |
| --- | --- |
| Startup route | `/chats` |
| Warm navigation targets | `/sessions`, `/agents`, `/settings/organization` |
| Samples per target | 5 |

## Steps

1. Load Chats directly and wait for Chats content and network idle.
2. Inspect page/RSC and API requests made since navigation began.
3. Hover Sessions, then use keyboard focus on Agents, inspecting requests after each intent.
4. Click Sessions and verify its page content and data render correctly.
5. Return to Chats and repeat five warm-navigation samples for Sessions, Agents, and Settings.
6. Return to Chats, click Settings, wait for `/settings/organization`, and inspect Settings page/RSC requests.

## Expected Result

- Chats startup makes no page/RSC or API request attributable only to an unrelated sidebar destination.
- Every sidebar `Link` disables automatic viewport prefetch.
- Hover or keyboard focus may prefetch only the intended destination; Settings remains fully opted out of intent prefetch.
- Click navigation, active state, feature-flag visibility, development-only sections, and keyboard accessibility remain correct.
- Settings loads only `/settings/organization`; no Settings child route is prefetched.
- Record total requests, page/RSC requests, API requests, unique routes, Chats readiness, and all five samples per target.
- On a stable environment, warm-navigation median is below 500 ms and p95 is below 800 ms; otherwise report the observed external or environment latency without hiding it.
