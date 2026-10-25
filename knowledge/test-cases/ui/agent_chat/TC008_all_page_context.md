---
type: Test Case
title: "TC008: Chat context across app pages"
description: "Verify page identity, filters, bounded loaded records and read-only behavior across every adopted app area."
tags: [ui, test-case, platform-chat, feature-flags]
---

# TC008: Chat context across app pages

## Preconditions

- Test organization with Adoption → Chat mode enabled, configured real model,
  and fictional records. Enable other area flags only in this test organization.
- Test owner/admin plus a restricted member; a second organization with distinct data.

## Steps

1. Open each permitted destination using Pages: Agents, Harnesses, Virtual Users,
   Chats, Playground, Sessions, Models, Web search, Knowledge, legacy Knowledge
   indexes where available, Memory, Skills, Capabilities, Plugins, Sandboxes,
   Evals, Observers, Reports, Monitoring, Approvals, Exposures, Durable and Settings.
   Include every visible tab/detail route; redirects must land on the canonical page.
2. On each page, ask “What page is beside this chat? Read its current context.”
   Inspect the tool result: route/label and entity identity must match the right page.
   Navigation must not create an authored conversation turn.
3. On a list, change search/status/tab or pagination without navigating away. Ask
   which filter/view is active. Verify the read reflects the actual React state and
   mounted record references. On Sessions also test source, agent, time and ownership
   filters. A second resource or inactive cached list must not appear in summaries.
4. Read a large list. Verify summary limits/truncation are reported and Chat remains
   usable. Full form values must fail explicitly if oversized, rather than truncating.
5. Visit Organization, Model defaults and Web search defaults. Ask for their current
   settings. Confirm the explicit public values match the controls. Asking to change
   an immediate-save setting must not trigger its onChange or a server-write fallback.
6. Visit read-only operational/history pages, approvals, credentials, imports and
   financial settings. Ask for an edit. The tool must refuse mutation and explain
   use of the existing page controls. No approval, purchase, grant or credential is changed.
7. Type a fixture secret in a credential/header input. Read page context. Inspect tool
   results: neither the fixture secret nor arbitrary page HTML or Chat transcript
   contents may be included. Do not use a real secret for this case.
8. Switch organization and repeat a read. No preceding organization's records, form
   binding or Chat composer state may remain. Repeat with a restricted member: hidden
   destinations remain absent and read-only forms reject patches.
9. From a read-only list, separately request an authorized saved-resource operation
   through normal platform commands, using a fictional record. It must retain the
   normal permission checks and must not be confused with editing an adjacent form.
10. Disable Chat mode and reload representative pages. Verify the ordinary shell and
   existing page controls, with no browser draft tools in managed Platform Chat.

## Expected result

Every authenticated app page shares the adopted workspace and an accurate binding.
Reads are explicit, bounded and scoped; page context never grants mutation authority.

## Notes

Public/shared/authentication/onboarding pages intentionally use their existing layouts.
Global operational queries without a known organization scope are omitted from cache
summaries. Chat may use permitted read-only platform tools for further requested detail.
