---
title: Model refusals stay distinct from provider failures
description: Responses refusals keep the same classification in streaming, JSON, batches and session traces, without automatic retries.
date: 2026-11-04
release: "0.51.0"
kind: fix
significance: minor
products: [framework, platform, cloud]
surfaces: [code, ui, agent]
---

Explicit model refusals through the Responses driver now end with the same typed
result whether the answer arrives as a stream, a JSON response or a batch result.
Streaming retains the complete reason instead of stopping at its first fragment;
JSON refusals no longer appear as successful assistant answers.

Session traces name the refusal, and a refusal after partial text still ends the
turn without retrying the model. The provider's reason follows the existing error
disclosure settings: standard mode shows a short refusal notice, while detailed
mode can show the bounded reason.
