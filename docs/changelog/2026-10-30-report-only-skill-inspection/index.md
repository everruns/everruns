---
title: Inspect skills and Yolop memory locally
description: Opt into heuristic skill and memory reports tied to content hashes and observed origin, without automatic blocking, removal or external scanning.
date: 2026-10-30
kind: feature
significance: minor
products: [framework, platform, yolop]
release: "0.50.1"
surfaces: [code, api, cli]
links:
  docs: [/features/skills/]
---

The `skills` capability can report heuristic findings in catalog entries and
activation results. Yolop's `memory` capability adds reports to approved writes
and selected recalls, including loaded notes. Enable each capability explicitly:

```json
{
  "report_only_inspection": true
}
```

Reports bind raw `SKILL.md` content, version, observed scope/path, and inspection
ruleset. Activation re-reads the artifact before reusing an inspected result.
The feature is off by default and is not in the published `0.49.0` crate.
Yolop requires an upstream dependency revision containing the shared inspector.

Inspection is local and report-only: it does not delete artifacts, contact
external scanners, or grant approval. It is heuristic, not guaranteed injection
protection. Memory disclosure inspects only visible titles, exposing no body
findings or hashes. Referenced skill files and substituted arguments are outside the scan.
See [Agent Skills](/features/skills/#optional-local-inspection) and
[Yolop memory configuration](https://github.com/everruns/yolop#skills-memory-and-hooks).
