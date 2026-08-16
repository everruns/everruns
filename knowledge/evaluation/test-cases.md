---
type: Specification
title: "Test Cases Specification"
description: "Manual test case format."
tags:
  - everruns
  - evaluation
---
# Test Cases Specification

## Abstract

Manual test case documentation format and organization.

## Requirements

### Location

`test_cases/` is split by target into `api/`, `cli/`, `ui/`, and `agents/` subfolders. API, UI, and agent test cases are organized into feature subfolders; CLI test cases may be kept flat or grouped into feature subfolders as needed.

```
test_cases/
├── api/          # HTTP API tests (curl, jq assertions)
│   ├── agents/
│   ├── sessions/
│   ├── ...
├── cli/          # CLI tests (everruns command invocations)
│   ├── TC001_files_ls_list_session_files.md
│   ├── ...
├── ui/           # Browser/UI tests (navigation, form input, clicks)
│   ├── admin_login/
│   ├── mcp_servers/
│   ├── ...
└── agents/       # End-to-end agent workflow tests (harness setup + agent run + assertions)
    ├── data_analyst/
    ├── ...
```

A feature may have test cases in multiple targets (e.g. `global_search` in both `api/` and `ui/`). Agent-workflow test cases in `agents/` exercise a complete harness + agent + session interaction and assert on events, tool calls, and final state.

### Format

Each test case file contains:

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

See also: [fail-rs-testing.md](fail-rs-testing.md) (failure injection), [agent-reliability-tests.md](../runtime-resources/agent-reliability-tests.md) (E2E reliability), [load-testing.md](../operations/load-testing.md) (performance), [evals.md](evals.md) (behavioral evals)
