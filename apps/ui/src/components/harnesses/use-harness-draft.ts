"use client";

// Draft state for the harness page's in-place edit mode.
//
// Same contract as the agent draft: every field starts as the saved value and
// is held until the page-level Save sends one update. Layers the user did not
// touch are omitted, because omitting leaves them unchanged server-side.
// system_prompt is special even among those: a harness may contribute no base
// prompt, so it is sent only when it changed — including "" to clear one.

import { useCallback, useMemo, useState } from "react";
import { normalizeCapabilityConfigs } from "@/components/agents/capability-config";
import { normalizeNetworkAccess } from "@/components/network-access-editor";
import {
  getFieldErrors,
  harnessFormSchema,
  parseTagList,
  type FieldErrors,
} from "@/lib/form-validation";
import { joinTags } from "@/lib/tags";
import type {
  AgentCapabilityConfig,
  Harness,
  InitialFile,
  NetworkAccessList,
  UpdateHarnessRequest,
} from "@/lib/api/types";
import type { ConversationStarter } from "@/lib/api/legacy-api-types";

export interface HarnessDraftFields {
  display_name: string;
  name: string;
  description: string;
  intro_markdown: string;
  short_description: string;
  system_prompt: string;
  tags: string;
  parent_harness_id: string;
  default_model_id: string;
}

const EMPTY_FIELDS: HarnessDraftFields = {
  display_name: "",
  name: "",
  description: "",
  intro_markdown: "",
  short_description: "",
  system_prompt: "",
  tags: "",
  parent_harness_id: "",
  default_model_id: "",
};

/** Fields edited in the Branding sheet; a validation error on one opens it. */
export const BRANDING_FIELDS = [
  "name",
  "display_name",
  "description",
  "intro_markdown",
  "short_description",
  "starters",
] as const;

function fieldsFromHarness(harness: Harness | undefined): HarnessDraftFields {
  if (!harness) return EMPTY_FIELDS;
  return {
    display_name: harness.display_name || "",
    name: harness.name,
    description: harness.description || "",
    intro_markdown: harness.intro_markdown || "",
    short_description: harness.short_description || "",
    system_prompt: harness.system_prompt || "",
    tags: joinTags(harness.tags),
    parent_harness_id: harness.parent_harness_id || "",
    default_model_id: harness.default_model_id || "",
  };
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

export type HarnessDraftResult =
  | { ok: true; request: UpdateHarnessRequest }
  | { ok: false; errors: FieldErrors };

export function useHarnessDraft(harness: Harness | undefined) {
  const initialFields = useMemo(() => fieldsFromHarness(harness), [harness]);
  const initialCapabilities = useMemo(
    () => normalizeCapabilityConfigs(harness?.capabilities),
    [harness?.capabilities],
  );
  const initialStarters = useMemo(() => harness?.starters ?? [], [harness?.starters]);
  const initialFiles = useMemo(() => harness?.initial_files ?? [], [harness?.initial_files]);
  const initialNetworkAccess = harness?.network_access ?? null;

  const [fieldChanges, setFieldChanges] = useState<Partial<HarnessDraftFields>>({});
  const [capabilities, setCapabilities] = useState<AgentCapabilityConfig[] | null>(null);
  const [starters, setStarters] = useState<ConversationStarter[] | null>(null);
  const [files, setFiles] = useState<InitialFile[] | null>(null);
  const [networkAccess, setNetworkAccess] = useState<NetworkAccessList | null>(null);
  const [errors, setErrors] = useState<FieldErrors>({});

  const fields = useMemo(
    () => ({ ...initialFields, ...fieldChanges }),
    [initialFields, fieldChanges],
  );

  const setField = useCallback((field: keyof HarnessDraftFields, value: string) => {
    setFieldChanges((prev) => ({ ...prev, [field]: value }));
    setErrors((prev) => ({ ...prev, [field]: undefined }));
  }, []);

  const values = {
    fields,
    capabilities: capabilities ?? initialCapabilities,
    starters: starters ?? initialStarters,
    files: files ?? initialFiles,
    networkAccess: networkAccess ?? initialNetworkAccess,
  };

  const capabilitiesChanged = !same(values.capabilities, initialCapabilities);
  const startersChanged = !same(values.starters, initialStarters);
  const filesChanged = !same(values.files, initialFiles);
  // `null`, `{}` and empty lists all mean "no restrictions on this layer".
  const networkAccessChanged =
    networkAccess !== null &&
    !same(normalizeNetworkAccess(networkAccess), normalizeNetworkAccess(initialNetworkAccess));
  const parentChanged =
    fieldChanges.parent_harness_id !== undefined &&
    fieldChanges.parent_harness_id !== initialFields.parent_harness_id;

  const isDirty =
    (Object.keys(fieldChanges) as (keyof HarnessDraftFields)[]).some(
      (key) => fieldChanges[key] !== initialFields[key],
    ) ||
    capabilitiesChanged ||
    startersChanged ||
    filesChanged ||
    networkAccessChanged;

  const reset = useCallback(() => {
    setFieldChanges({});
    setCapabilities(null);
    setStarters(null);
    setFiles(null);
    setNetworkAccess(null);
    setErrors({});
  }, []);

  /** Validates the draft and builds the single update request Save sends. */
  const buildRequest = (): HarnessDraftResult => {
    const parsed = harnessFormSchema.safeParse({ ...fields, starters: values.starters });
    if (!parsed.success) {
      const fieldErrors = getFieldErrors(parsed.error);
      setErrors(fieldErrors);
      return { ok: false, errors: fieldErrors };
    }
    const data = parsed.data;
    // Omit an unchanged prompt so a promptless harness is not rewritten to "".
    // Send "" when a previous prompt was cleared.
    const prompt = data.system_prompt ?? "";
    const systemPromptChanged = prompt.trim() !== initialFields.system_prompt.trim();
    return {
      ok: true,
      request: {
        name: data.name,
        display_name: data.display_name,
        description: data.description,
        intro_markdown: data.intro_markdown || null,
        short_description: data.short_description || null,
        ...(startersChanged && { starters: values.starters }),
        ...(parentChanged && { parent_harness_id: data.parent_harness_id || null }),
        tags: parseTagList(data.tags),
        default_model_id: data.default_model_id,
        ...(systemPromptChanged && { system_prompt: prompt }),
        ...(capabilitiesChanged && { capabilities: values.capabilities }),
        ...(filesChanged && { initial_files: values.files }),
        ...(networkAccessChanged && { network_access: networkAccess }),
      },
    };
  };

  return {
    ...values,
    initialNetworkAccess,
    errors,
    isDirty,
    nameChanged: fields.name !== initialFields.name,
    setField,
    setCapabilities,
    setStarters,
    setFiles,
    setNetworkAccess,
    reset,
    buildRequest,
  };
}

export type HarnessDraft = ReturnType<typeof useHarnessDraft>;
