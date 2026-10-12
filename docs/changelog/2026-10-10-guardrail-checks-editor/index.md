---
title: "Pick and configure guardrail checks in the agent editor"
description: "The Guardrails settings panel lets authors add presets or single checks and set their stage and action, and model-backed checks can now judge the final reply."
date: 2026-10-10
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui, agent]
links:
  docs: [/capabilities/guardrails/]
  prs: [everruns/everruns#4459, everruns/everruns#4542]
visual: "Agent capability settings with the Guardrails editor showing the preset gallery and a check's stage and action (/agents/<id>), cropped to the panel"
---

**In the UI:** the Guardrails capability settings on an agent or harness is now
an editor instead of a raw form. Add a gallery preset to append its checks, or
add a single check and set its rule, stage and what happens on a match. Choose
whether a policy judge or moderation check is answered by the utility model or
the organization's decision model. The mode reads **Active** or **Advisory**.

**For agents:** `llm_judge` and `mcp` checks are now valid on the output stage.
They run once on the finalized assistant message, with the same caps, advisory
downgrade and fail-open behavior as the tool stages.

See [Guardrails](/capabilities/guardrails/) and [#4459](https://github.com/everruns/everruns/pull/4459), [#4542](https://github.com/everruns/everruns/pull/4542).
