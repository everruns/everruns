import { z } from "zod";

export const PAGE_READ_TOOL = "read_page_draft";
export const PAGE_PATCH_TOOL = "patch_page_draft";
export const MAX_PAGE_DRAFT_SIZE = 256 * 1024;

// Display overlays have the same bounded shape as the page editor. They cannot
// alter stable names, instructions, capabilities or other execution settings.
export const entityLocalizationsDraftSchema = z
  .record(
    z
      .string()
      .max(64)
      .regex(/^[a-zA-Z]{2,8}(?:[-_][a-zA-Z0-9]{1,8})*$/),
    z.strictObject({
      display_name: z
        .string()
        .max(255)
        .refine((v) => !v.includes("\0"))
        .nullable()
        .optional(),
      description: z
        .string()
        .max(2000)
        .refine((v) => !v.includes("\0"))
        .nullable()
        .optional(),
    }),
  )
  .refine((entries) => {
    const locales = Object.keys(entries);
    return (
      locales.length <= 64 &&
      new Set(locales.map((key) => key.replaceAll("_", "-").toLowerCase())).size === locales.length
    );
  }, "Use at most 64 distinct locales.");

// Form strings stay strings, including tag lists and token limits. Save retains
// its normal validation; Chat can repair one field in an otherwise invalid form.
export const agentDraftPatchSchema = z
  .object({
    name: z.string().optional(),
    display_name: z.string().optional(),
    description: z.string().optional(),
    intro_markdown: z.string().optional(),
    short_description: z.string().optional(),
    system_prompt: z.string().optional(),
    tags: z.string().optional(),
    harness_id: z.string().optional(),
    default_model_id: z.string().optional(),
    communication: z.enum(["direct", "explicit"]).optional(),
    max_output_tokens: z.string().optional(),
    localizations: entityLocalizationsDraftSchema.optional(),
    capabilities: z
      .array(
        z
          .object({ ref: z.string().min(1), config: z.record(z.string(), z.unknown()).default({}) })
          .strict(),
      )
      .max(100)
      .optional(),
    starters: z
      .array(z.object({ text: z.string(), icon: z.string().nullable().optional() }).strict())
      .max(20)
      .optional(),
    initial_files: z
      .array(
        z
          .object({
            path: z.string(),
            content: z.string(),
            encoding: z.enum(["text", "base64"]).default("text"),
            is_readonly: z.boolean().default(false),
          })
          .strict(),
      )
      .max(100)
      .optional(),
    network_access: z
      .object({ allowed: z.array(z.string()).optional(), blocked: z.array(z.string()).optional() })
      .strict()
      .optional(),
  })
  .strict()
  .refine((patch) => Object.keys(patch).length > 0, "Provide at least one form field");

export type AgentDraftPatch = z.infer<typeof agentDraftPatchSchema>;
export const agentDraftPatchJsonSchema = {
  ...z.toJSONSchema(agentDraftPatchSchema, { unrepresentable: "any" }),
  minProperties: 1,
};

export interface PageDraftDescriptor {
  binding: string;
  revision: string;
  dirty: boolean;
  writable: boolean;
}

export interface PageDraftSnapshot<
  T extends object = Record<string, unknown>,
> extends PageDraftDescriptor {
  entity_id: string;
  entity_type?: string;
  values: T;
  patch_schema?: Record<string, unknown>;
  forms?: Record<string, { label: string; writable: boolean; dirty: boolean }>;
  // Shown for context. Template snapshots require the dedicated sandbox editor.
  sandbox_policy?: unknown;
}

export interface PageDraftAdapter<T extends object = Record<string, unknown>> {
  readonly binding?: string;
  describe: () => PageDraftDescriptor;
  read: (binding: string) => PageDraftSnapshot<T>;
  patch: (binding: string, revision: string, changes: unknown) => PageDraftSnapshot<T>;
  previewPatch?: (binding: string, revision: string, changes: unknown) => PageDraftSnapshot<T>;
}

export function assertDraftSize(value: unknown) {
  if (new TextEncoder().encode(JSON.stringify(value)).length > MAX_PAGE_DRAFT_SIZE)
    throw new Error("This page draft exceeds the Chat size limit. Edit it in the page instead.");
}
