import { z } from "zod";
import { agentDraftPatchSchema } from "./page-draft";
import { draftText } from "./use-page-form-draft";

export const virtualUserCreateDraftSchema = z.strictObject({
  usage: z.enum(["service", "end_user"]),
  name: draftText,
  description: draftText,
  locale: draftText,
  timezone: draftText,
});
export const harnessCreateDraftSchema = z.strictObject({
  display_name: draftText,
  name: draftText,
  description: draftText,
  system_prompt: draftText,
  parent_harness_id: draftText,
  default_model_id: draftText,
  tags: draftText,
  capabilities: agentDraftPatchSchema.shape.capabilities.unwrap(),
  initial_files: agentDraftPatchSchema.shape.initial_files.unwrap(),
  network_access: agentDraftPatchSchema.shape.network_access.unwrap(),
});
export const evalCreateDraftSchema = z.strictObject({
  name: draftText,
  description: draftText,
  model_override: draftText,
  target_type: z.enum(["session", "app"]),
  harness_id: draftText,
  agent_id: draftText,
  app_id: draftText,
  system_prompt: draftText,
  model_id: draftText,
  tags: z.array(draftText).max(100),
});
export const evalCaseDraftSchema = z.strictObject({
  name: draftText,
  description: draftText,
  messages: draftText,
  scorer_type: z.enum(["contains", "not_contains", "regex", "tool_called"]),
  scorer_value: draftText,
});
export const scheduleCreateDraftSchema = z.strictObject({
  name: draftText,
  description: draftText,
  cron_expression: draftText,
  target_type: z.enum(["workflow", "activity"]),
  target_name: draftText,
  enabled: z.boolean(),
});
export const scheduleEditDraftSchema = z.strictObject({
  description: draftText,
  cron_expression: draftText,
  max_concurrent: z.string().regex(/^(?:[1-9]\d*)?$/),
  catch_up_missed: z.boolean(),
  max_catch_up: z.string().regex(/^(?:[1-9]\d*)?$/),
});
export const collectionDraftSchema = z.strictObject({
  name: draftText,
  description: draftText,
  mode: z.enum(["off", "lexical", "hybrid"]),
  model: draftText,
  indexer: draftText,
  rules: draftText,
});
export const tracingRoutingDraftSchema = z.strictObject({
  mode: z.enum(["inherit", "add", "replace", "off"]),
  destination_ids: z.array(z.string().uuid()).max(8),
});
export const sourceDraftSchema = z.strictObject({
  kind: z.enum(["github", "git"]),
  coordinates: draftText.refine(
    (value) => !value.includes("://") || isPublicDraftLocation(value),
    "Use a repository URL without credentials.",
  ),
  branch: draftText,
  folder: draftText,
  root: draftText,
  identity: draftText,
});
export const providerTraceDraftSchema = z.strictObject({
  enabled: z.boolean(),
  generation_url_template: draftText.refine(
    isPublicDraftLocation,
    "Use a public trace URL without credentials.",
  ),
  session_url_template: draftText.refine(
    isPublicDraftLocation,
    "Use a public trace URL without credentials.",
  ),
});
export const providerAdvancedDraftSchema = z.strictObject({ cache_diagnostics: z.boolean() });
export const providerConnectDraftSchema = z.strictObject({
  name: draftText,
  custom_endpoint: z.boolean(),
  base_url: draftText.refine(isPublicDraftLocation, "Use an endpoint without credentials."),
});
export const mcpMetadataDraftSchema = z.strictObject({
  name: draftText,
  description: draftText,
  url: draftText.refine(isPublicDraftLocation, "Use an MCP URL without credentials."),
  protocol_mode: z.enum(["auto", "2025-03-26", "2025-06-18", "2026-07-28"]),
  elicitation_policy: z.enum(["url", "url_and_form", "none"]),
  tool_timeout: draftText,
});
export const declarativeDraftSchema = z.strictObject({
  name: draftText,
  displayName: draftText,
  description: draftText,
  skills: z
    .array(
      z.strictObject({
        id: draftText,
        name: draftText,
        description: draftText,
        instructions: draftText,
        files: z
          .array(z.strictObject({ id: draftText, path: draftText, content: draftText }))
          .max(100),
        userInvocable: z.boolean(),
        disableModelInvocation: z.boolean(),
      }),
    )
    .max(100),
  files: z
    .array(
      z.strictObject({
        id: draftText,
        path: draftText,
        content: draftText,
        access: z.enum(["readonly", "readwrite"]),
      }),
    )
    .max(100),
});

// URL-valued fields are optional context, never a route for exposing embedded
// credentials. Blank/unfinished input stays editable manually; only ordinary
// public HTTP(S) locations may enter a Chat form snapshot.
export function isPublicDraftLocation(value: string): boolean {
  if (!value) return true;
  try {
    const url = new URL(value);
    return (
      ["http:", "https:"].includes(url.protocol) &&
      !url.username &&
      !url.password &&
      !url.hash &&
      [...url.searchParams.entries()].every(
        ([key, item]) =>
          ["id", "response_id", "session_id", "turn_id", "model"].includes(key) &&
          /^\{(?:response_id|session_id|turn_id|model)\}$/.test(item),
      )
    );
  } catch {
    return false;
  }
}
