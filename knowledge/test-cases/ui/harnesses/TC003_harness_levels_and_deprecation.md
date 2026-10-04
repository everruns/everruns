---
type: Test Case
title: "TC003: Harness levels and Generic deprecation"
description: "Verify minimal selection, explicit example imports and preserved legacy values."
tags:
  - ui
  - harnesses
---
# TC003: Harness levels and Generic deprecation

Open Harnesses in light and dark mode. Verify Base, Conversation, Worker Base and Worker are listed with their parent relationships. Generic is initially hidden. Choose Show deprecated and verify Generic is active and marked deprecated.

Create an agent and open its harness selector. Generic is initially hidden, then appears with Show deprecated. Edit a legacy Generic agent and verify its selected value remains readable without enabling the toggle.

Open agent examples. Dad Jokes displays Conversation before import. Import it and verify its effective preview exposes time but no bash, file, storage or subagent tools. Verify Worker Base exposes bash without subagents; Worker exposes spawning and task controls. Member-role assignment of Worker's high-risk capabilities is rejected.

Expected: the smallest appropriate preset is visible and selected, legacy selections remain executable, and preview matches runtime tools.
