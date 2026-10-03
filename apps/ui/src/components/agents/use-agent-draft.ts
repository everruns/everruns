"use client";

// Draft state for the agent page's in-place edit mode.
//
// The agent page shows and edits an agent in one layout, so the draft lives
// beside the read view instead of on its own route. Every field starts as the
// saved value; a change is held here until the page-level Save sends one
// update, so a prompt edit and the capability change that goes with it land
// together. Fields the user did not touch keep the request shape the old edit
// route sent: harness, capabilities, starters, files and network access are
// omitted unless they changed, because omitting leaves them as they are
// server-side (an inherited harness must not be pinned by an unrelated save).

import { useCallback, useMemo, useState } from "react";
import { normalizeCapabilityConfigs } from "@/components/agents/capability-config";
import { normalizeNetworkAccess } from "@/components/network-access-editor";
import {
  agentFormSchema,
  getFieldErrors,
  parseTagList,
  type FieldErrors,
} from "@/lib/form-validation";
import { joinTags } from "@/lib/tags";
import type {
  Agent,
  AgentCapabilityConfig,
  EnvironmentSet,
  InitialFile,
  NetworkAccessList,
  UpdateAgentRequest,
} from "@/lib/api/types";
import type { ConversationStarter } from "@/lib/api/legacy-api-types";

export interface AgentDraftFields {
  display_name: string;
  name: string;
  description: string;
  intro_markdown: string;
  short_description: string;
  system_prompt: string;
  tags: string;
  harness_id: string;
  default_model_id: string;
}

const EMPTY_FIELDS: AgentDraftFields = {
  display_name: "",
  name: "",
  description: "",
  intro_markdown: "",
  short_description: "",
  system_prompt: "",
  tags: "",
  harness_id: "",
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

function fieldsFromAgent(agent: Agent | undefined): AgentDraftFields {
  if (!agent) return EMPTY_FIELDS;
  return {
    display_name: agent.display_name || "",
    name: agent.name,
    description: agent.description || "",
    intro_markdown: agent.intro_markdown || "",
    short_description: agent.short_description || "",
    system_prompt: agent.system_prompt,
    tags: joinTags(agent.tags),
    harness_id: agent.effective_harness?.id || agent.harness_id || "",
    default_model_id: agent.default_model_id || "",
  };
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

export type AgentDraftResult =
  | { ok: true; request: UpdateAgentRequest }
  | { ok: false; errors: FieldErrors };

export function useAgentDraft(agent: Agent | undefined) {
  const initialFields = useMemo(() => fieldsFromAgent(agent), [agent]);
  const initialCapabilities = useMemo(
    () => normalizeCapabilityConfigs(agent?.capabilities),
    [agent?.capabilities],
  );
  const initialStarters = useMemo(() => agent?.starters ?? [], [agent?.starters]);
  const initialFiles = useMemo(() => agent?.initial_files ?? [], [agent?.initial_files]);
  const initialEnvironments = useMemo(() => agent?.environments ?? null, [agent?.environments]);
  const initialNetworkAccess = agent?.network_access ?? null;

  const [fieldChanges, setFieldChanges] = useState<Partial<AgentDraftFields>>({});
  const [capabilities, setCapabilities] = useState<AgentCapabilityConfig[] | null>(null);
  const [starters, setStarters] = useState<ConversationStarter[] | null>(null);
  const [files, setFiles] = useState<InitialFile[] | null>(null);
  const [networkAccess, setNetworkAccess] = useState<NetworkAccessList | null>(null);
  // undefined means untouched; null is an intentional clear sent to the API.
  const [environments, setEnvironments] = useState<EnvironmentSet | null | undefined>(undefined);
  const [errors, setErrors] = useState<FieldErrors>({});

  const fields = useMemo(
    () => ({ ...initialFields, ...fieldChanges }),
    [initialFields, fieldChanges],
  );

  const setField = useCallback((field: keyof AgentDraftFields, value: string) => {
    setFieldChanges((prev) => ({ ...prev, [field]: value }));
    setErrors((prev) => ({ ...prev, [field]: undefined }));
  }, []);

  const values = {
    fields,
    capabilities: capabilities ?? initialCapabilities,
    starters: starters ?? initialStarters,
    files: files ?? initialFiles,
    networkAccess: networkAccess ?? initialNetworkAccess,
    environments: environments === undefined ? initialEnvironments : environments,
  };

  const capabilitiesChanged = !same(values.capabilities, initialCapabilities);
  const startersChanged = !same(values.starters, initialStarters);
  const filesChanged = !same(values.files, initialFiles);
  // `null`, `{}` and empty lists all mean "no restrictions on this layer".
  const networkAccessChanged =
    networkAccess !== null &&
    !same(normalizeNetworkAccess(networkAccess), normalizeNetworkAccess(initialNetworkAccess));
  const environmentsChanged =
    environments !== undefined && !same(environments, initialEnvironments);
  const harnessChanged =
    fieldChanges.harness_id !== undefined && fieldChanges.harness_id !== initialFields.harness_id;

  const isDirty =
    (Object.keys(fieldChanges) as (keyof AgentDraftFields)[]).some(
      (key) => fieldChanges[key] !== initialFields[key],
    ) ||
    capabilitiesChanged ||
    startersChanged ||
    filesChanged ||
    networkAccessChanged ||
    environmentsChanged;

  const reset = useCallback(() => {
    setFieldChanges({});
    setCapabilities(null);
    setStarters(null);
    setFiles(null);
    setNetworkAccess(null);
    setEnvironments(undefined);
    setErrors({});
  }, []);

  /** Validates the draft and builds the single update request Save sends. */
  const buildRequest = (): AgentDraftResult => {
    const parsed = agentFormSchema.safeParse({ ...fields, starters: values.starters });
    if (!parsed.success) {
      const fieldErrors = getFieldErrors(parsed.error);
      setErrors(fieldErrors);
      return { ok: false, errors: fieldErrors };
    }
    const data = parsed.data;
    return {
      ok: true,
      request: {
        name: data.name,
        display_name: data.display_name,
        description: data.description,
        intro_markdown: data.intro_markdown || null,
        short_description: data.short_description || null,
        ...(startersChanged && { starters: values.starters }),
        system_prompt: data.system_prompt,
        tags: parseTagList(data.tags),
        ...(harnessChanged && { harness_id: data.harness_id }),
        default_model_id: data.default_model_id,
        ...(capabilitiesChanged && { capabilities: values.capabilities }),
        ...(filesChanged && { initial_files: values.files }),
        ...(networkAccessChanged && { network_access: networkAccess }),
        ...(environmentsChanged && { environments: values.environments }),
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
    setEnvironments,
    reset,
    buildRequest,
  };
}

export type AgentDraft = ReturnType<typeof useAgentDraft>;
