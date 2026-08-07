import { z } from "zod";
import type { ConnectionFormField } from "@/lib/api/types";
import { LOCALE_OPTIONS, TIMEZONE_OPTIONS } from "./locale-data";

export type FieldErrors = Partial<Record<string, string>>;

const VALID_NAME_PATTERN = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const localeValues = new Set(LOCALE_OPTIONS.map((option) => option.value));
const timezoneValues = new Set(TIMEZONE_OPTIONS.map((option) => option.value));

function trimInput(value: unknown): string {
  return typeof value === "string" ? value.trim() : "";
}

function requiredString(label: string) {
  return z.preprocess(trimInput, z.string().min(1, `${label} is required`));
}

function optionalString() {
  return z.preprocess(trimInput, z.string()).transform((value) => value || undefined);
}

function optionalSelection(validValues: Set<string>, label: string) {
  return z
    .preprocess(trimInput, z.string())
    .transform((value) => value || undefined)
    .refine((value) => !value || validValues.has(value), `Select a valid ${label.toLowerCase()}`);
}

export function getFieldErrors(error: z.ZodError): FieldErrors {
  const fieldErrors: FieldErrors = {};

  for (const issue of error.issues) {
    const field = issue.path[0];
    if (typeof field === "string" && !fieldErrors[field]) {
      fieldErrors[field] = issue.message;
    }
  }

  return fieldErrors;
}

export function parseTagList(value: string): string[] {
  return value
    .split(",")
    .map((tag) => tag.trim())
    .filter((tag) => tag.length > 0);
}

export const agentFormSchema = z.object({
  name: requiredString("Name").refine(
    (value) => VALID_NAME_PATTERN.test(value),
    "Name must contain only lowercase letters, numbers, and hyphens",
  ),
  display_name: optionalString(),
  description: optionalString(),
  system_prompt: requiredString("System prompt"),
  harness_id: requiredString("Harness"),
  default_model_id: optionalString(),
  tags: z.preprocess(trimInput, z.string()).optional().default(""),
});

export const harnessFormSchema = z.object({
  name: requiredString("Name").refine(
    (value) => VALID_NAME_PATTERN.test(value),
    "Name must contain only lowercase letters, numbers, and hyphens",
  ),
  display_name: optionalString(),
  description: optionalString(),
  // Optional: a harness may contribute no base prompt and rely on the parent
  // harness, agent, session, and capability layers.
  system_prompt: optionalString(),
  parent_harness_id: optionalString(),
  default_model_id: optionalString(),
  tags: z.preprocess(trimInput, z.string()).optional().default(""),
});

export const agentIdentityFormSchema = z.object({
  name: requiredString("Name"),
  description: optionalString(),
  locale: optionalSelection(localeValues, "Locale"),
  timezone: optionalSelection(timezoneValues, "Timezone"),
});

export const mcpServerFormSchema = z
  .object({
    name: requiredString("Name"),
    description: optionalString(),
    url: requiredString("URL").refine(
      (value) => z.url().safeParse(value).success,
      "URL must be a valid absolute URL",
    ),
    auth_mode: z.enum(["none", "api_key", "oauth"]),
    api_key: optionalString(),
  })
  .superRefine((value, ctx) => {
    if (value.auth_mode === "api_key" && !value.api_key) {
      ctx.addIssue({
        code: "custom",
        path: ["api_key"],
        message: "API key is required",
      });
    }
  });

/**
 * Edit schema for an existing MCP server. Covers the mutable identity fields
 * (name, description, url) plus protocol compatibility. Auth mode and the API
 * key are managed via their own dedicated flows and are intentionally omitted.
 */
export const mcpServerEditFormSchema = z.object({
  name: requiredString("Name"),
  description: optionalString(),
  url: requiredString("URL").refine(
    (value) => z.url().safeParse(value).success,
    "URL must be a valid absolute URL",
  ),
  protocol_mode: z.enum(["auto", "2025-03-26", "2025-06-18", "2026-07-28"]),
});

export const apiKeySecretSchema = z.object({
  api_key: requiredString("API key"),
});

export const knowledgeIndexFormSchema = z.object({
  name: requiredString("Name"),
  description: optionalString(),
  embedding_model_id: requiredString("Embedding model"),
});

export function createConnectionFormSchema(fields: ConnectionFormField[]) {
  const shape = Object.fromEntries(
    fields.map((field) => [
      field.name,
      field.required ? requiredString(field.label) : optionalString(),
    ]),
  );

  return z.object(shape);
}
