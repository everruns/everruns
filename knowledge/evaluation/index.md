# Evaluation, testing, and reporting

* [Evals](evals.md) - User-facing behavioral evals.
* [Eval experiments](experiments.md) - Datasets with versions, experiments over variants, paired comparison, and the CI gate.
* [Multistep eval cases](multistep.md) - Simulated users that carry a conversation from a persona and goal, and tool fixtures with canned responses.
* [External Evaluation Results Publishing](external-results-publishing.md) - Publishing externally executed eval results into Everruns and remaining reporting work.
* [Agent Checks](agent-checks.md) - Advisory agent config checks (lint, LLM analysis, health checks).
* [Suggested fixes for findings](suggested-fixes.md) - Inspector-proposed instruction changes, checked against the agent's evals and applied only with a person's approval.
* [Self-healing](self-healing.md) - Per-org choice of what Everruns does on its own when a finding opens: automatic pause of repeating schedules, recorded and undoable.
* [Online Evals (Observers)](online-evals.md) - Online evals (Observers) over production sessions (proposed).
* [SWE-bench Lite on Everruns](swe-bench-lite.md) - SWE-bench Lite evaluation harness.
* [Reporting](reporting.md) - Async reporting.
* [Dataset Export Specification](dataset-export.md) - Reward-labeled trajectory dataset export from eval runs.
* [ATIF (Agent Trajectory Interchange Format) Adoption](atif-adoption.md) - ATIF (Agent Trajectory Interchange Format) export/import adoption.
* [Reporting Backends Evaluation](reporting-backends.md) - Phase 3 reference evaluation.
* [fail-rs Testing Specification](fail-rs-testing.md) - Failure injection testing with fail-rs.

* [Unit-test quality review](unit-test-quality.md) - Individual test review evidence and completion criteria.
