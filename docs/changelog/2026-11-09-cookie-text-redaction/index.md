---
title: Session exports hide cookies in command text
description: Cookie headers and assignments inside command strings are now redacted in session exports and hook argument previews, without changing execution.
date: 2026-11-09
kind: fix
significance: minor
products: [platform]
surfaces: [api]
---

Session exports and hook-rewritten argument previews now hide `Cookie` and
`Set-Cookie` values embedded in free-form text, including shell commands and
serialized arguments. Previously, cookie fields in structured objects were
redacted, but a cookie header inside a command string could remain visible.

For example, `curl -H 'Cookie: sid=example; csrf=example'` is shown as
`curl -H 'Cookie: [REDACTED]'`. Multiple cookies, quoted values and cookie
assignments are covered. Redaction runs before preview truncation and changes
only the exported or recorded copy; the command passed to the tool is unchanged.
