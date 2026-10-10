---
type: Specification
title: "Feature Flags"
description: "Rollout grades govern feature availability, organisation defaults, and configuration authority."
tags:
  - everruns
  - security
---
# Feature Flags

A feature's rollout grade determines where it is available, whether it starts enabled,
and who may change its organisation override. The deployment grade remains a separate
concept: it describes the running deployment, not a feature's maturity.

## Policy

Local-development features cannot be activated on hosted deployments by either a tenant
or a platform operator. Internal enrolment is an explicit platform decision for individual
organisations. Adoption features let organisation owners/admins or platform operators opt in. Production
features start enabled for every organisation, while owners and admins may opt out.
An off feature is unavailable regardless of existing organisation records or actor.

A feature that is already opt-in, because someone has to add a capability, a connection
or a channel before anything happens, gets no feature flag. A flag on top would gate the
same opt-in twice. Flags are for surfaces that appear on their own (pages, panels,
behaviour changes) or that a deployment must keep unavailable (off/dev/internal grades).
Removed flag names left in the organisation table are ignored: resolution only reads
names the catalog defines.

Feature defaults live in the [catalog](../../crates/server/src/records/feature_flags.rs).
An environment override replaces a feature's rollout grade at process startup. Invalid
values disable the feature rather than silently promoting it. The shared
[grade policy](../../crates/contracts/src/runtime/feature_flag_grade.rs) owns parsing, availability,
defaults, and tenant configuration authority.

Dev features start enabled on a local development deployment and may be disabled there.
No organisation override makes a dev feature available on another deployment grade.
Promotion changes the default and configuration owner immediately; an existing explicit
organisation override remains in effect whenever the new grade permits availability.

## Organisation overrides

The existing [organisation flag table](../../crates/server/migrations/046_org_feature_flags.sql)
stores explicit overrides in both directions. A missing record inherits the grade default;
a stored false is a durable production opt-out, not an instruction to remove the record.
Patch operations change only named flags. The default-organisation seeder must not enrol
features: defaults belong to the grade policy, so bootstrap cannot bypass internal/off.

[Resolution](../../crates/server/src/services/org_feature_flags.rs) reads durable overrides
on every enforcement path. A replica-local cache must not delay revocation. Storage
failures deny feature access instead of treating unknown overrides as production defaults.

## Configuration authority

Tenant settings expose only locally available development, adoption, and production
features. Tenant updates require an organisation owner/admin and reject internal and off
features in both directions. Platform settings expose every grade; the platform
update route permits internal and adoption enrolments. Adoption has shared
configuration authority: the latest explicit override wins, regardless of which
permitted actor wrote it. Operators cannot change production preferences or promote
an off/local-only feature. Platform permission is independent of tenant membership
and never follows from ownership of any organisation (EVE-1206).

The [HTTP API](../../crates/server/src/api/org_feature_flags.rs) owns wire shapes and route
permissions. The public deployment flag map describes whether routes/capabilities can
exist, while the organisation flag map returns the effective booleans. Settings include
the resolved grade, inherited default, optional stored override, effective state, and the
viewer's configuration authority. The [UI flag hooks](../../apps/ui/src/hooks/use-org-feature-flags.ts)
displays effective switch state so production defaults appear on even without a record.

## Execution boundary

Hosted capability registration uses deployment availability. Internal/adoption registration
does not authorise use: the server filters capabilities using organisation-effective flags
before loading a worker snapshot, command dispatch, or assigning configurations.
Infrastructure capabilities follow this same policy when promoted beyond their default off grade.

Integration capabilities and connectors use the same flags; there is no separate dev-only switch.
A plugin names an optional flag, and the integration crate that owns the plugin declares that
flag's label and default grade next to it. The
[hosted integration catalog](../../crates/capabilities/src/integrations_catalog.rs) collects those declarations,
and the platform catalog lists them with its own, so they share settings, overrides, and
organisation enrolment. A plugin without a flag is generally available. Connection providers
are filtered by the organisation-effective flag when listed and when a connection is created.

Payment management honours the organisation flag rather than overriding it based on router
availability. Payment execution re-reads durable overrides immediately before spending,
so a loaded tool cannot bypass a revocation. Existing domain command gates remain the authority for their feature-owned
operations. A flag's scope is the surface it owns; reporting aggregation and existing session
recordings remain infrastructure rather than being deleted when a UI feature is unavailable.

The runtime receives resolved booleans, not rollout-management records. The
[core registration decisions](../../crates/contracts/src/runtime/execution_features.rs) and the
[platform catalog](../../crates/server/src/records/feature_flags.rs) share the grade policy;
only the hosted platform resolves durable organisation overrides.

## Success bars

- Every grade obeys its availability/default/authority rules for every deployment grade.
- Off and non-local dev features ignore stale true overrides.
- Production defaults can be disabled durably without affecting another organisation.
- Tenant and platform mutations obey their grade authority; adoption accepts both actors.
- Registration and runtime capability filtering agree, including infrastructure flags.
- UI switches reflect effective values and member controls remain read only.

The grade matrix is covered in core/platform unit tests, ownership and persistence in the
[server policy tests](../../crates/server/src/services/org_feature_flags.rs), and settings
behaviour in the [UI tests](../../apps/ui/src/__tests__/features-settings-page.test.tsx).
