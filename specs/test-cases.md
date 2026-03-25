# Test Cases Specification

## Abstract

Manual test case documentation format and organization.

## Requirements

### Location

`test_cases/` is split by target into `api/`, `cli/`, and `ui/` subfolders. API and UI test cases are organized into feature subfolders; CLI test cases may be kept flat or grouped into feature subfolders as needed.

```
test_cases/
├── api/          # HTTP API tests (curl, jq assertions)
│   ├── agents/
│   ├── sessions/
│   ├── ...
├── cli/          # CLI tests (everruns command invocations)
│   ├── TC001_files_ls_list_session_files.md
│   ├── ...
└── ui/           # Browser/UI tests (navigation, form input, clicks)
    ├── admin_login/
    ├── mcp_servers/
    ├── ...
```

A feature may have test cases in multiple targets (e.g. `global_search` in both `api/` and `ui/`).

### Format

Each test case file contains:

- **Description**: What the test verifies
- **Preconditions**: Required setup and environment
- **Test Data**: Input values (table format)
- **Steps**: Numbered actions to perform
- **Expected Result**: Success criteria

### Naming

`TC###_short_description.md`

Numbering is **per leaf folder** — each feature folder (or target root when flat, e.g. `cli/`) starts at TC001 independently. Do not share numbering across folders or targets.

Examples:
- `TC001_success_login.md`
- `TC002_invalid_credentials.md`
- `TC015_session_timeout.md`

### When to Create

New features should have corresponding test cases documenting expected behavior and acceptance criteria.
