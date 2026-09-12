---
type: Specification
title: "Test Cases Specification"
description: "Manual test case format."
tags:
  - everruns
  - evaluation
  - test-case
---
# Test Cases Specification

## Abstract

Manual test case documentation format and organization.

## Requirements

### Location

Manual test cases live in `knowledge/test-cases/`, as concepts of this knowledge bundle. They are
split by target into `api/`, `cli/`, `ui/`, and `agents/` subfolders. API, UI, and agent test cases
are organized into feature subfolders; CLI test cases may be kept flat or grouped into feature
subfolders as needed.

```
knowledge/test-cases/
├── index.md      # Domain index: targets and this specification
├── format.md     # This specification
├── api/          # HTTP API tests (curl, jq assertions)
│   ├── index.md
│   ├── agents/
│   ├── sessions/
│   ├── ...
├── cli/          # CLI tests (everruns command invocations)
│   ├── index.md
│   ├── TC001_files_ls_list_session_files.md
│   ├── ...
├── ui/           # Browser/UI tests (navigation, form input, clicks)
│   ├── index.md
│   ├── admin_login/
│   ├── mcp_servers/
│   ├── ...
└── agents/       # End-to-end agent workflow tests (harness setup + agent run + assertions)
    ├── index.md
    ├── data_analyst/
    ├── ...
```

A feature may have test cases in multiple targets (e.g. `global_search` in both `api/` and `ui/`).
Agent-workflow test cases in `agents/` exercise a complete harness + agent + session interaction and
assert on events, tool calls, and final state.

Cases sit in the bundle so an agent reaches them by the same progressive disclosure as every other
concept: [`knowledge/index.md`](../index.md) names the domain, the domain index names the targets, a
target index names its features, and a feature index lists its cases with one-line descriptions.
Nobody has to read 200 case files to find the relevant one.

### OKF conformance

Each case is an OKF concept, so it carries frontmatter with `type: Test Case`, a `title` that
repeats the `TC###: ...` heading, and a one-sentence `description` taken from its Description
section. Every folder carries an `index.md` that lists the concepts and immediate subfolders beside
it, and nothing deeper. [`knowledge/knowledge-contract.md`](../knowledge-contract.md) owns the rules;
`just check-okf` enforces them.

Supporting material that is not a concept (screenshots and other captured evidence) lives in an
`evidence/` subfolder beside the cases it belongs to, linked from that folder's index.

### Format

Each test case file opens with an `# TC###: <Feature> - <Short title>` heading, then contains:

- **Description**: What the test verifies
- **Preconditions**: Required setup and environment
- **Test Data**: Input values (table format)
- **Steps**: Numbered actions to perform
- **Expected Result**: Success criteria

### Naming

`TC###_short_description.md`

Numbering is **per leaf folder**: each feature folder (or target root when flat, e.g. `cli/`) starts at TC001 independently. Do not share numbering across folders or targets.

Examples:
- `TC001_success_login.md`
- `TC002_invalid_credentials.md`
- `TC015_session_timeout.md`

### When to Create

New features should have corresponding test cases documenting expected behavior and acceptance criteria.

## Related Testing Specs

See also: [fail-rs-testing.md](../evaluation/fail-rs-testing.md) (failure injection), [agent-reliability-tests.md](../runtime-resources/agent-reliability-tests.md) (E2E reliability), [load-testing.md](../operations/load-testing.md) (performance), [evals.md](../evaluation/evals.md) (behavioral evals)
