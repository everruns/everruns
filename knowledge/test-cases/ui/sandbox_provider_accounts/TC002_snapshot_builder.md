---
type: Test Case
title: "TC002: Sandbox snapshot builder"
description: "Build a real provider snapshot from a generic script, inspect notes, save a guarded revision and verify cleanup and runtime policy."
tags:
  - everruns
  - test-case
  - ui
  - sandbox
---
# TC002: Sandbox snapshot builder

## Preconditions

Canonical local stack; an administrator; a named Daytona Organization provider
account; disposable provider quota. Capture resource identifiers for cleanup.

## Steps

1. Create an editable Daytona template bound to that exact account. Choose a
   runtime network policy and a bootstrap command. Save the configuration.
2. Open **Build snapshot**. Confirm a generic install script editor is shown;
   there is no application catalog. Managed templates have no build action.
3. Write a script installing a small tool and a non-secret software-notes file.
   Expand **Verification and build access**, enter a command checking the installed
   tool and the absolute metadata-file path. Choose build network permissions
   independently of runtime permissions.
4. Start the build. Follow its real steps and bounded log. Reload while it runs;
   the attempt remains visible. A second concurrent build in the organization
   must be rejected. No Session, personal workspace or workload credential is
   attached to the builder.
5. Wait for a reusable snapshot and verification on fresh compute. Confirm both
   disposable provider sandboxes disappear and cleanup is complete before saving.
6. Inspect captured notes, then **Save new revision**. Confirm the snapshot is
   selected, revision increments once, bootstrap and runtime policy are preserved,
   and existing Agent/Session pins retain their old artifact.
7. Select the new template for an Agent and create a Session. Inspect its Sandbox:
   the applied build and author-supplied notes match the exact artifact/account.
   Launch it and verify the installed tool and notes file are present.
8. In a second attempt, edit the source template while building. Applying the
   result must fail; it cannot overwrite the newer revision. Discard that unused
   snapshot and confirm provider deletion.
9. Run a script that exits unsuccessfully, then another long-running script and
   cancel it. Each must finish failed/cancelled and remove its disposable resources.
   A provider error, timeout or server restart must not automatically replay an
   install script or falsely report readiness. Cleanup failure remains visible.
10. Confirm API reads and writes using a different organization cannot access the
    build, apply/discard it, or resolve its software notes. An artifact referenced
    with another provider account must not inherit provenance.
11. Check narrow and mobile layouts, light/dark themes, translated copy, keyboard
    controls, plain-text rendering of malicious HTML in notes, and clean browser
    console. Editing configuration disables building until saved.

## Expected result

The install script produces a qualified reusable filesystem snapshot. Applying
uses existing immutable revisions, with runtime access and old pins preserved.
Failed, cancelled and unused builds are accounted for and cleaned up.

## Cleanup

Discard unused artifacts through Everruns. For an applied test artifact, end its
Sessions, remove test Agent/template references, and explicitly delete only the
recorded test snapshot at the provider. Stop the local stack.
