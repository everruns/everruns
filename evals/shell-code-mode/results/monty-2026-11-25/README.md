# Monty Code Mode integration evidence

One trial per selected cell, using GPT-6.2 Sol. This is integration evidence,
not a model or performance ranking. Framework/MCP used OpenRouter; hosted smokes
used OpenAI directly. Concurrent compilation makes the recorded wall times
unsuitable for performance comparisons. See [summary.json](summary.json) for
source and executable hashes, per-case scores, usage and trace archive hashes.

## Framework task comparison

| Surface | Valid task observations | Approval allow | Approval reject |
| --- | ---: | --- | --- |
| Native | 7/8 | Failed to apply after approval | Passed |
| Bashkit CLI | 7/8 | Failed to apply after approval | Passed |
| Monty | 8/8 | Saved continuation resumed; one reservation and one apply | Saved continuation resumed; no apply |

These observations combine the unaffected original cases with a fresh weather
trial on each surface. The original 24 trials scored 19/24: three false negatives
rejected “Lisbon, Portugal” because the old fixture required exactly “Lisbon.”
The corrected fixture permits that country qualification while rejecting other
cities/countries or the wrong number of days. The three fresh weather trials
passed. Original traces and [the original dataset](dataset-initial.jsonl) remain
available; the original scores have not been rewritten.

The two remaining failures are model behavior: both native and CLI agents stopped
after the actual approval decision instead of retrying the pending operation and
verifying it. Their traces include the approval message. No unauthorized apply
or duplicate reservation occurred. This small sample does not establish that
Monty is generally more reliable. Surface-specific operating instructions are
part of the study; Monty approval cases explicitly request one program and saved
resume. All arms use the same synthetic task, tools and approval decisions.

## MCP and hosted boundaries

The final MCP smoke passed **8/8**, with no infrastructure failures or skips:
read-only ticket lookup and staging rollback, 50 tools, one server, across native,
CLI, hybrid and Monty. The original run scored 7/8 because the old scorer joined
nested results only to Bash output. The corrected scorer joins nested calls to
the fixture's own call journal by exact operation and arguments. The complete
matrix was rerun; original and fresh reports are separate. Larger catalogs and
repeated performance comparisons remain unqualified for Monty.

The hosted workspace smoke passed **6/6**: native, CLI and Monty on both local
Server/worker and agentd. It requires a single direct file write, authoritative
read-back and a `stat_file` call through the selected execution surface.

The first agentd Monty attempt incorrectly asked direct-only file tools to run
inside Python and failed. The corrected prompt respected that contract; its
first grade missed `read_file`'s numbered-line envelope. `agentd-monty-graded`
corrects that grade using the same captured events as `agentd-monty-v2`; it is
not another model run. Both failed harness attempts are retained.

## Qualification limits

All local Monty runs used **interpreter-only isolation**, explicitly selected for
synthetic fixtures. The Linux Landlock probe returned `ENOSYS`. These results do
not qualify the production launcher. The required CI job exercises the hardened
worker, kernel restrictions and the real Engine approval/resume test; its kernel
qualification was not executed successfully in this environment.

The hosted smokes do not test approval recovery after executor replacement,
Server restart, or agentd Raft leader movement. Framework approval tests and
private-journal unit tests do not establish those deployment properties.

Focused validation passed: 15 ordinary Framework study tests, the real Monty
Engine allow/reject test, 33 MCP study tests, three hosted-grader tests, and 49 CI
policy tests. Docs and local knowledge checks passed. Framework Clippy passed
strictly; supplemental MCP Clippy passed with its existing fixture `dead_code`
and `collapsible_if` warnings allowed. The upstream knowledge linter was not
installed; the repository's local check passed.

## Reports and reproduction

- `framework-live.jsonl.gz`: chained tools and both approval decisions, all three surfaces.
- `framework-remaining.jsonl.gz`: the other five tasks, including the original weather false negatives.
- `framework-weather-retest.jsonl.gz`: three fresh trials after the weather expectation fix.
- `mcp-live.jsonl.gz`: original eight-trial MCP smoke, including the missing-result-evidence failure.
- `mcp-retest.jsonl.gz`: fresh eight-trial matrix with fixture-journal grading.
- `agentd-*.json.gz` and `server-*.json.gz`: hosted boundary events and grading lineage.

Archives preserve synthetic task/tool evidence and usage, while omitting provider
reasoning blocks and encrypted reasoning payloads. Source hashes describe the
uncommitted implementation after validation; initial and final executable hashes
are separate. Reporting comments and grader fixes changed during the run.

Use the [study instructions](../../README.md) and
[MCP runner](../../../mcp-scale/README.md) with a qualified Monty worker and private
journal storage. The recorded target is `openrouter/openai/gpt-6.2-sol`, with
`EVERRUNS_EVAL_TRIALS=1`. The MCP run also sets `EVERRUNS_EVAL_CONCURRENCY=1` and
uses `--filter catalog_size=50,servers=1, --sample readonly-ticket,staged-rollback`.
The trailing comma prevents matching the ten-server arm. Request more repetitions
and representative workloads before using results to choose a default surface.
