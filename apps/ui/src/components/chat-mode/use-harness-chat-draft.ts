"use client";

import { z } from "zod";
import type { HarnessDraft, HarnessDraftFields } from "@/components/harnesses/use-harness-draft";
import { agentDraftPatchSchema, entityLocalizationsDraftSchema } from "./page-draft";
import { draftText, usePageFormDraft } from "./use-page-form-draft";

const schema = z.strictObject({
  name: draftText,
  display_name: draftText,
  description: draftText,
  intro_markdown: draftText,
  short_description: draftText,
  system_prompt: draftText,
  tags: draftText,
  parent_harness_id: draftText,
  default_model_id: draftText,
  localizations: entityLocalizationsDraftSchema,
  capabilities: agentDraftPatchSchema.shape.capabilities.unwrap(),
  starters: agentDraftPatchSchema.shape.starters.unwrap(),
  initial_files: agentDraftPatchSchema.shape.initial_files.unwrap(),
  network_access: agentDraftPatchSchema.shape.network_access.unwrap(),
});

export function useHarnessChatDraft({
  id,
  label,
  updatedAt,
  draft,
  writable,
  available,
  beginEdit,
}: {
  id: string;
  label: string;
  updatedAt?: string;
  draft: HarnessDraft;
  writable: boolean;
  available: boolean;
  beginEdit: () => void;
}) {
  return usePageFormDraft({
    entityId: id,
    entityType: "harness",
    label,
    schema,
    baseline: updatedAt ?? id,
    values: {
      ...draft.fields,
      capabilities: draft.capabilities,
      starters: draft.starters,
      initial_files: draft.files,
      network_access: draft.networkAccess ?? {},
      localizations: draft.localizations ?? {},
    },
    dirty: draft.isDirty,
    writable,
    enabled: available,
    apply: (values) => {
      for (const field of Object.keys(draft.fields) as Array<keyof HarnessDraftFields>)
        if (values[field] !== draft.fields[field]) draft.setField(field, values[field]);
      const differs = (left: unknown, right: unknown) =>
        JSON.stringify(left) !== JSON.stringify(right);
      if (differs(values.capabilities, draft.capabilities))
        draft.setCapabilities(values.capabilities);
      if (differs(values.starters, draft.starters)) draft.setStarters(values.starters);
      if (differs(values.initial_files, draft.files)) draft.setFiles(values.initial_files);
      if (differs(values.localizations, draft.localizations ?? {}))
        draft.setLocalizations(values.localizations);
      // Null means inherit; normalizing it for reading must not create an
      // explicit empty override when Chat edited an unrelated field.
      if (differs(values.network_access, draft.networkAccess ?? {}))
        draft.setNetworkAccess(values.network_access);
      beginEdit();
    },
  });
}
