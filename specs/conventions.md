# Development Conventions

## Abstract

Development conventions for testing, commits, PRs, and API error handling.

## Testing

### Unit Tests

Inline `#[cfg(test)]` modules for:
- Serialization/deserialization validation
- Error response formats
- Pure functions and transformations
- Business logic without external dependencies

### Integration Tests

Add to `tests/integration_test.rs` for:
- New API endpoints
- Complex multi-service workflows
- End-to-end verification

### When to Add Tests

- New features: Always
- Bug fixes: Regression tests when feasible
- Refactoring: Ensure existing tests pass; add if gaps found
- Security fixes: Tests verifying the fix

### Running Tests

```bash
# Unit tests
cargo test

# Integration tests (requires API + Worker running)
cargo test -p everruns-control-plane --test integration_test -- --test-threads=1
```

### Manual Test Cases

Location: `test_cases/` organized by feature.

**Format:**
- Description: What the test verifies
- Preconditions: Required setup
- Test Data: Input values (table format)
- Steps: Numbered actions
- Expected Result: Success criteria

**Naming:** `TC###_short_description.md` (e.g., `TC001_success_login.md`)

## Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org):

```
<type>[optional scope]: <description>
```

**Types:** feat, fix, docs, style, refactor, perf, test, chore, ci

**Examples:**
```
feat(api): add agent versioning endpoint
fix(workflow): handle timeout in run execution
```

## Pull Requests

**Title:** Conventional Commits format.

**Merge strategy:** Squash and Merge.

**Body template** (from `.github/pull_request_template.md`):

```markdown
## What
Clear description of the change.

## Why
Problem or motivation.

## How
High-level approach.

## Risk
- Low / Medium / High
- What can break

## Checklist
- [ ] Tests added or updated
- [ ] Backward compatibility considered
```

## API Error Handling

- **Never expose internal error details.** Return generic `500 Internal Server Error` with "Internal server error".
- **Always log server-side:** `tracing::error!()` before returning generic response.
- Only return safe, user-facing messages: "Not found", "Invalid request", "Internal server error".

Example:
```rust
let result = state.db.some_operation().await.map_err(|e| {
    tracing::error!("Failed to perform operation: {}", e);
    StatusCode::INTERNAL_SERVER_ERROR
})?;
```
