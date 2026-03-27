# TC003: Evals - Create New Eval

## Description

Verify that a new eval can be created with required fields (name, agent, harness).

## Preconditions

- Server running (`just start-dev`)
- User logged in
- Feature flag `evals` enabled (`FEATURE_EVALS=true`)
- At least one agent exists
- At least one harness exists

## Test Data

| Field | Value |
|-------|-------|
| Name | `Test Greeting Eval` |
| Description | `Verifies the agent greets users correctly` |
| Agent | (first available agent) |
| Harness | (first available harness) |

## Steps

1. Navigate to `/evals`
2. Click "New Eval" button
3. Verify the create form loads at `/evals/new`
4. Fill in Name: `Test Greeting Eval`
5. Fill in Description: `Verifies the agent greets users correctly`
6. Select an agent from the dropdown
7. Select a harness from the dropdown
8. Leave Model Override as "Use agent default"
9. Click "Create Eval"
10. Wait for redirect to the eval detail page

## Expected Result

| Check | Expected |
|-------|----------|
| Form fields | Name, Description, Agent, Harness, Model Override, Tags all visible |
| Agent dropdown | Lists available agents |
| Harness dropdown | Lists available harnesses |
| Submit disabled | "Create Eval" button disabled until agent and harness selected |
| Redirect | After creation, redirects to `/evals/{eval_id}` |
| Detail page | Shows eval name, description, status "active", agent name |
| Cases tab | Shows "No test cases yet" empty state |
| Runs tab | Shows "No runs yet" empty state |
