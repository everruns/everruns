# TC006: Global Chat - Verify Agents in Agents Page

## Description

After creating agents via global chat, verify they appear correctly in the Agents listing page with proper names, status, and details.

## Preconditions

- Server running (`just start-dev`)
- User logged in
- 10 agents created via global chat (TC003 completed)

## Test Data

| Agent Name | Expected Status |
|------------|----------------|
| Code Reviewer | active |
| Data Analyst | active |
| Writing Editor | active |
| Math Tutor | active |
| DevOps Helper | active |
| Security Auditor | active |
| API Designer | active |
| Test Writer | active |
| Debug Assistant | active |
| Doc Generator | active |

## Steps

1. Navigate to `/agents`
2. Observe the agents list
3. Click on each agent to view its detail page
4. Verify name, system prompt, and status for each

## Expected Result

| Check | Expected |
|-------|----------|
| All 10 visible | All 10 agents appear in the list |
| Names match | Each agent name matches what was requested |
| Status | All show `active` status |
| System prompts | Each has a non-empty system prompt relevant to its purpose |
| Detail pages load | Clicking each agent navigates to `/agents/{id}` without errors |
