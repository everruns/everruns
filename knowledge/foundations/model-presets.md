---
type: Specification
title: "Model Presets"
description: "Named sets of models, one per job, that agents can take their models from instead of the organization defaults."
tags:
  - everruns
  - foundations
  - models
---
# Model Presets

## Abstract

A model preset is a named set of models, one for each job an organization assigns a model to:
Default, Chat, Coding, Research, Small and Decision. The organization defaults stay the baseline
for every agent. A preset is an optional named set on top: an agent can take its models from a
preset (for example "Gemini") instead of from the defaults, and a preset can be copied into the
defaults in one action. Presets are the agent-side half of choosing models for spawned helpers
(see [Subagents](../runtime-resources/subagents.md)).

## Requirements

### Kinds

- **Automatic** (built in, id `automatic`): for each job, the first usable model that is its
  provider's curated pick for that job, preferring Anthropic, then OpenAI, then other providers
  in the order the org lists its models. Research follows the same order as the organization
  research model's Automatic mode.
- **Provider presets** (built in, read only, id `provider:<driver>`): one per connected driver
  that declares curated picks (`ProviderRecommendedModels` in
  `crates/contracts/src/recommended_models.rs`) and has a usable model. A job whose curated model
  is not usable falls back to the provider's next recommended usable model (cheapest first for
  Small and Decision) and is listed in `fallback_jobs` so the page can mark it.
- **Custom presets**: created blank or by duplicating any preset (`from`). A job left empty uses
  the organization default for that job.

Built-ins are computed on every read from the org's models and the drivers' picks, never stored,
so refreshed picks reach every org at once. Only models that can serve turns now count: enabled
chat models on an active, shared provider with credentials. Personal providers never appear.

### Resolution

`turn_model_id` (`crates/server/src/domains/organizations/research_model/mod.rs`) is the single
resolver for control-plane turns, cell definitions and the session's displayed model:

1. The session's model, else the agent's pinned model.
2. Research role: the agent's preset Research job, else the organization research model.
3. The agent's preset: Coding on the coding harness, else Default.
4. The harness default model.
5. On the coding harness, the organization Coding default.
6. The built-in Chat agent's organization Chat model.
7. Otherwise the organization default model.

A preset that no longer exists or leaves a job empty falls through to the next step.

### Rules

- Custom preset ids are slugs from the name, unique among live presets, never `automatic`.
- A custom preset cannot be deleted while live agents take their models from it.
- Built-ins are read only; duplicate one to customize it.
- Copying a preset into the defaults changes only the jobs the preset fills. The Decision default
  is a calibrated decision-service model, so a preset's Decision model is copied only when it
  qualifies; otherwise the job is reported as skipped.
- Agent packages carry the preset by id. A destination without that preset imports the agent on
  its organization defaults instead of refusing it.

### Surfaces

Every operation is a domain command (`crates/server/src/domains/model_presets/commands.rs`), so
REST (`/v1/model-presets`), MCP, the in-session platform capability and the
`everruns model-presets` CLI share one schema. Creating, changing and deleting a custom preset
are recorded in change history. The Models page shows presets on their own tab (Models, Presets,
Providers, Defaults); the agent page's Model section picks "Models from".

### Helper models

An agent with the Subagents capability lists the models its subagents may run on as *helper
keys* in that capability's config (`helper_models`, `helper_model_pick`; contract in
`crates/contracts/src/runtime/helper_models.rs`). Built-ins: `same` (the parent's model),
`fast` (Small job), `coding` (Coding job), `deep` (Research job) and `second_opinion` (Default
job of the first provider preset whose provider differs from the parent's). Owners repoint,
remove or add keys; a target is always a job, a preset job, an org model, or another provider's
preset, never a raw model name, so the owner controls cost and which providers see the data.

- `spawn_agent` gets `model`, an enum of the keys with their "when to use" lines, built when the
  session starts. An unknown key is a tool error that lists the keys. Only `same` hides the field.
- Who picks: the spawning model names a key (none means `same`), or on **Auto** the Decision
  model reads the brief and picks; a missing or failing decision service keeps `same`.
- The control plane resolves the key when it creates the child
  (`crates/server/src/domains/model_presets/helper.rs`) and stores it as the child's session
  model, so it sticks for follow-ups. `same` copies the person's explicit parent choice (session
  model, else the latest message's model). Unusable targets leave the child on ordinary
  resolution rather than failing the spawn.
- Cells ask the same resolution over the `helper_model` host call and write the answer into the
  child's definition; `same` copies the parent's definition model. Framework (local) runs every
  key on the parent's model.
- The task spec records `model`, `model_pick`, `model_pick_reason` and `model_id`; the session's
  Subagents tab shows them as a chip. Hand-offs keep the member agent's own model. Fixed
  blueprints refuse a key.

## Design Decisions

| Decision | Why |
|---|---|
| Org defaults stay the baseline, presets sit on top | Existing settings keep working with no migration; a preset is opt-in per agent. |
| Built-ins computed, not stored | Curated picks change with releases; a stored copy would go stale. |
| Provider presets read only | They follow the curated picks; customizing means duplicating. |
| Preset jobs fall through when empty | A custom preset can change one job and inherit the rest. |
| Helper keys, not model names, at spawn | The owner decides cost and data exposure; the model only chooses among approved keys. |
| Helper resolution on the control plane | One answer on every plane, inside the org's usable catalog. |
| A preset is the simple form of a model router | One route per job, one model each. When routers ship, a job can point at a route. |
