## What changed
Describe the change functionally — what behavior changes and its impact on users
or callers. Lead with outcomes; don't walk through code locations, the diff shows
where and how. Keep any code-level notes short and specific.

## Why
Problem or motivation.

## Before / After
Show the effect with evidence. Include before and after whenever behavior changes —
CLI/API output, logs, metrics, or screenshots/recordings for UI (attach working media
when possible). For changes with no observable behavior (pure refactor, docs), say so.

## Risk
- Low / Medium / High
- What can break

## Security
- Threat categories reviewed: (e.g. TM-AUTH, TM-API, or "No security-relevant code changes" with justification)
- Findings and resolutions:

## Follow-ups
List anything intentionally deferred with a one-line rationale, or write "No follow-ups." Prefer implementing in-scope work in this PR over deferring it.

## Checklist
- [ ] Tests added or updated
- [ ] Backward compatibility considered
- [ ] Security review performed against relevant threat model categories
- [ ] Final CI ran checks affected by this PR
- [ ] All review comments addressed (code change or written reasoning)
