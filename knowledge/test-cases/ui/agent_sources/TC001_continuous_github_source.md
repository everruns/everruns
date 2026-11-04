---
type: Test Case
title: "TC001: Continuous GitHub source ownership"
description: Verify reviewed source application, continuous updates, last-good retention, read-only ownership and disconnect behavior.
tags: [ui, agents, github, synchronization]
---

# TC001: Continuous GitHub source ownership

## Description

Verify reviewed source application, continuous updates, last-good retention,
read-only ownership and disconnect behavior. The owning contract is
[Agent sources](../../../runtime-resources/agent-sources.md).

## Preconditions

- Run the canonical local stack with database-backed persistence.
- In authenticated mode, use an organization member with Agent management
  permission and an Agent GitHub App installed on the fixture repository.
- Use a new active Agent and a dedicated repository branch so prior data cannot
  satisfy assertions. Record its Agent ID, branch and initial commit.
- For ownership denials, also use a member without Agent management permission
  and a user belonging only to another organization. Send their own credentials
  without fallback cookies.

## Test Data

| Item | Value |
| --- | --- |
| Package path | `agents/support/` with one canonical manifest and instructions |
| Folder-name variation | Repeat preview with a package folder named `agents/support.md/` |
| Initial Agent instructions | A unique local marker |
| First source revision | A different instruction marker and a declared runtime file |
| Next source revision | A third instruction marker and changed runtime file content |
| Invalid revision | A manifest referencing a nonexistent capability |

## Steps

1. Open the Agent. Choose **Source → Sync from GitHub**, select the repository,
   branch and package path, then preview. Confirm the preview shows the expected
   instruction and file changes. Cancel once and re-read the Agent and source
   status to verify that preview did not persist changes.
2. Preview again and enable continuous sync. Verify the original Agent ID is
   retained, the applied commit matches the reviewed revision, the declared file
   is present in its exported package, and Edit is unavailable.
3. Try authored-field updates and package import through the API against this
   Agent. Verify rejection and unchanged exported content. Verify users without
   management permission and users in another organization cannot connect,
   rebind or disconnect the source; re-read the original binding afterwards.
   With Chat mode adopted, ask Chat to patch this Agent. Verify its page draft
   is read-only, no patch succeeds, and no server save occurs as a fallback.
4. Advance the branch to the next valid revision. Without requesting manual
   sync, wait up to two minutes for the applied commit and Agent export to change.
   Verify instructions and file content both match that revision.
5. Advance the branch to the invalid revision. Wait up to two minutes for a
   visible sync error. Re-read the export and applied commit; both must still
   match the last valid revision, and Edit must remain unavailable.
6. Correct the branch and verify automatic recovery within two minutes. Exercise
   **Sync now** separately and verify that it checks the existing binding.
7. Disconnect source sync. Verify the source status is absent, the last applied
   definition is unchanged and local editing works again. Advance the repository
   once more, wait two minutes, and verify no further changes apply to this Agent.

## Expected Result

The source applies complete valid packages to the same Agent continuously,
rejects unauthorized local writes, retains the last valid package on failure,
and stops applying changes after disconnect. Destination credentials and
publication controls retain their existing local behavior.

## Two-way sync

1. Grant Contents write on the GitHub App, accept it on the installation, and select
   **Sync with GitHub** in the source dialog. Preview and enable the reviewed version.
2. Edit instructions in Everruns and save. Expect queued status, then a new commit
   on the selected branch, retaining the external instruction file and other files.
3. Push a repository-only change, then save another Agent edit. Expect both commits
   retained. Push a package change with no local edits; expect it applied normally.
4. Change the package on both sides before reconciliation. Expect a visible conflict
   with the local edit retained and no force push. Review the GitHub version and
   apply it explicitly to resolve; alternatively disconnect to keep local edits.
5. Protect the branch against direct writes. Expect a visible error and retained
   local edits; permit the App and retry to deliver the existing queued commit.
6. In Chat mode, verify draft edits and save are available for a two-way source.
   Reconnect in pull mode and verify authored editing becomes unavailable again.
7. Disconnect with queued work. Expect the current local definition retained and
   no new attempts, with the confirmation explaining in-flight requests.
