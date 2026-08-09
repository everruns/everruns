# TC006: Global Chat - Verify Agents in Agents Page

## Description

After creating agents via global chat, verify they appear correctly in the Agents listing page with proper display names, slugs, status, and details.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- 10 agents created via global chat (TC003 completed)

## Test Data

| Display Name | Slug | Expected Status |
|------------|------|----------------|
| Code Reviewer | `code-reviewer` | active |
| Data Analyst | `data-analyst` | active |
| Writing Editor | `writing-editor` | active |
| Math Tutor | `math-tutor` | active |
| DevOps Helper | `devops-helper` | active |
| Security Auditor | `security-auditor` | active |
| API Designer | `api-designer` | active |
| Test Writer | `test-writer` | active |
| Debug Assistant | `debug-assistant` | active |
| Doc Generator | `doc-generator` | active |

## Steps

1. Navigate to `/agents`
2. Observe the agents list
3. Verify each agent card shows display name prominently and slug in monospace underneath
4. Click on each agent to view its detail page
5. Verify display name, slug, system prompt, and status for each

## Expected Result

| Check | Expected |
|-------|----------|
| All 10 visible | All 10 agents appear in the list |
| Display names | Each card shows the human-readable display name (e.g. "Code Reviewer") as the title |
| Slugs shown | Each card shows the slug (e.g. `code-reviewer`) in monospace below the display name |
| Status | All show `active` status |
| System prompts | Each has a non-empty system prompt relevant to its purpose |
| Detail pages load | Clicking each agent navigates to `/agents/{id}` — detail page shows display name as heading with slug underneath |
