# Results file template

Write to `test_cases/ui/MANUAL_TEST_RESULTS_<YYYY-MM-DD>.md`.

```markdown
# Manual UI Test Results - <YYYY-MM-DD>

## Environment

- **Auth Mode**: <admin|full>
- **Stack**: <components running>
- **PORT_PREFIX**: <value>
- **Browser**: Chromium (headless, via agent-browser)

## Test Summary

| Category | Tests | Pass | Fail/Partial | Issues |
|----------|-------|------|--------------|--------|
| ... | ... | ... | ... | ... |
| **Total** | **N** | **N** | **N** | **N** |

## Detailed Results

### <Category> (N/M PASS)

- **TC001 <Name>**: PASS|FAIL|PARTIAL — <what happened, one line>

## Issues Found

### Issue #N (<Severity>): <Title>

- **Severity**: Low|Medium|High|Info
- **Steps**: how to reproduce
- **Expected**: what should happen
- **Actual**: what happened
- **Impact**: user-facing consequence
```
