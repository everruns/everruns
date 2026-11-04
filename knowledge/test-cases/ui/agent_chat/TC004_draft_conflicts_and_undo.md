---
type: Test Case
title: "TC004: Draft conflicts and guarded Undo"
description: "Verify that stale Chat patches and draft Undo cannot overwrite newer manual Agent edits."
tags: [ui, test-case, agents, platform-chat]
---

# TC004: Draft conflicts and guarded Undo

## Preconditions

- Chat workspace enabled; operator can edit a disposable Agent.
- Real model configured; browser network/trace tools available.
- Record the saved instructions before starting. Use distinct manual and Chat sentences.

## Steps

1. Add an unsaved manual sentence, then ask Chat for an instruction edit.
2. After `read_page_draft` has captured the form but before the patch arrives,
   type a second manual sentence. Slow the tool-result request if needed to make
   the timing repeatable. Confirm the read result predates that second sentence;
   otherwise repeat this step rather than claiming a stale-patch test.
3. Verify the old-revision patch fails without changing any form field. Chat may
   read again and apply a fresh patch that preserves both manual sentences.
4. After a successful Chat edit, manually alter a Chat-marked field. Press that
   field's draft Undo. Confirm a conflict is shown and the newer manual text stays.
5. Revert only that later manual alteration to the Chat result and retry Undo.
   Confirm the preceding manual draft returns and unrelated fields remain.
6. Make one Chat edit, manually refine it, then request a second Chat edit to the
   same field. Undo the second edit. Confirm the intervening manual refinement
   is the new Undo baseline.
7. Make Chat edits to two fields, manually alter one, then use Undo draft edits.
   Confirm the inverse fails atomically rather than partially overwriting fields.

## Expected result

- Stale revisions return a conflict and make no form or saved-definition changes.
- Retrying requires a new live read; no fallback server save bypasses the conflict.
- Draft Undo restores only its own preceding values, and refuses later manual edits.
- The saved Agent stays unchanged until ordinary Save is used.
