---
title: PR reviewer and security scanner agent templates for GitHub
description: Two guided agent templates use the agent's GitHub App to review pull requests with inline comments and to run a weekly scan that files GitHub issues.
date: 2026-10-01
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui]
release: "0.34.0"
links:
  docs: [/how-to/set-up-review-and-security-agents/]
  prs: [everruns/everruns#3980]
---

Two guided templates build on the per-agent GitHub App. The PR reviewer reviews
pull requests and posts deduplicated inline comments. The weekly security
scanner files fingerprinted issues, so a repeat finding updates the same issue,
and it can open draft fix pull requests, which is off by default.

The templates add GitHub review, issue upsert and pull request tools, enforced
by the GitHub capability's configuration, and a guided setup page.

See [Set up review and security agents](/how-to/set-up-review-and-security-agents/).
PR [#3980](https://github.com/everruns/everruns/pull/3980).
