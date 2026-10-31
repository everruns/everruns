---
title: Yolop preserves shell and background failure evidence
description: Yolop stops on unhandled shell failures, keeps command diagnostics visible to the agent, and reports blocking background tasks when print-mode waiting expires.
date: 2026-10-30
kind: fix
significance: notable
products: [yolop]
surfaces: [cli, agent]
links:
  prs: [everruns/yolop#902]
---

Yolop now stops shell scripts on unhandled command failures and preserves failed
pipeline producer statuses. A command such as `false | tee proof.log` reports
failure even when `tee` succeeds. Expected nonzero results must be handled
explicitly with `if` or `||`.

Failed commands reach the agent as failed tool events with bounded diagnostics,
including when the command text is long.
Repeated argument errors keep their repair instructions visible. When print-mode
background waiting expires, the error names the blocking tasks, attempts and
ownership evidence without dumping task specifications.

Git history rewriting requires explicit human approval for the specific action,
including when ordinary approval prompts are disabled. A request to ship does
not grant permission to force-push.
