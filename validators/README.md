# Validators

A validator is an executable non-functional requirement: one command that
answers "does the system meet this goal?" with a score from 0 to 100, the
evidence behind it, how sure that evidence is, and the cheapest next step
that would raise it. Each validator lives in its own folder and owns
nothing but its scoring and adapters; the load tooling it drives stays
where it is.

| Validator | Goal |
|---|---|
| [agents-1m](agents-1m/README.md) | 1,000,000 agents in parallel on data plane v2 |

Design and reasoning: `knowledge/operations/validators.md`.
