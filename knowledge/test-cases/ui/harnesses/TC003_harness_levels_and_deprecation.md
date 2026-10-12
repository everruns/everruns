---
type: Test Case
title: "TC003: Harness tree and deprecation"
description: "Verify minimal selection, explicit example imports and preserved legacy values."
tags:
  - ui
  - harnesses
---
# TC003: Harness tree and deprecation

Open Harnesses in light and dark mode. Verify Base, Conversation, Worker, Bashkit Worker and Sandbox Worker are listed with their parent relationships (Conversation and Worker on Base, both shell workers on Worker). Generic and Worker Base are initially hidden. Choose Show deprecated and verify both are active and marked deprecated.

Create an agent and open its harness selector. Generic is initially hidden, then appears with Show deprecated. Edit a legacy Generic agent and verify its selected value remains readable without enabling the toggle.

Open agent examples. Dad Jokes displays Conversation before import. Import it and verify its effective preview exposes time but no bash, file, storage or subagent tools. Verify Worker exposes files, spawning and task controls but no bash; Bashkit Worker adds bash. Saving a Bashkit Worker agent with a Daytona sandbox policy is rejected, and saving a Sandbox Worker agent with a Bashkit-only policy is rejected. Member-role assignment of Worker's high-risk capabilities is rejected.

Expected: the smallest appropriate preset is visible and selected, legacy selections remain executable, and preview matches runtime tools.
