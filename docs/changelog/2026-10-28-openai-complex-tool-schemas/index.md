---
title: Complex OpenAI tool schemas use non-strict mode
description: Tools with deeply nested objects or large property counts now use OpenAI's non-strict mode automatically, while execution still validates arguments.
date: 2026-10-28
kind: fix
significance: minor
products: [platform, framework, cloud]
surfaces: [agent, code]
---

OpenAI tool requests now check schema complexity before enabling strict mode.
Schemas with more than 10 nested object levels or 5,000 total properties use the
existing non-strict fallback, avoiding provider rejections for those limits.
Schemas within the limits retain strict conversion.

This applies to both Chat Completions and Responses. The original tool schema
still controls argument validation when the tool executes. Non-strict calls keep
their arguments unchanged, including null values; the fallback does not make an
invalid argument valid. No configuration change is required.
