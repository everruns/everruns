# Profile sources

Numbers-only inputs to `capture.py` that are safe to commit: per-turn
duration, model calls, context size and output size, copied from
claude-code-remote `list_events` `result` events. Never put transcript text,
prompts, answers, code, file names or commands here.

| File | Session | Turns |
|---|---|---|
| `bashkit-issue.events.json` | a bashkit issue fix (2026-09-21): PR checks, CI waits, merge | 4 |

Claude Code transcripts themselves stay where they are (for example
`~/.claude/projects/` on the Mac mini); capture reads them in place.
