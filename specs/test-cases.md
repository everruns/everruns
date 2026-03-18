# Test Cases Specification

## Abstract

Manual test case documentation format and organization.

## Requirements

### Location

`test_cases/` split by target into `api/` and `ui/` subfolders, then organized by feature.

```
test_cases/
├── api/          # HTTP API tests (curl, jq assertions)
│   ├── agents/
│   ├── sessions/
│   ├── ...
└── ui/           # Browser/UI tests (navigation, form input, clicks)
    ├── admin_login/
    ├── mcp_servers/
    ├── ...
```

A feature may have test cases in both `api/` and `ui/` (e.g. `global_search`, `scheduled_tasks`).

### Format

Each test case file contains:

- **Description**: What the test verifies
- **Preconditions**: Required setup and environment
- **Test Data**: Input values (table format)
- **Steps**: Numbered actions to perform
- **Expected Result**: Success criteria

### Naming

`TC###_short_description.md`

Examples:
- `TC001_success_login.md`
- `TC002_invalid_credentials.md`
- `TC015_session_timeout.md`

### When to Create

New features should have corresponding test cases documenting expected behavior and acceptance criteria.
