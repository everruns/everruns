import { z } from "zod";

export const PAGE_READ_TOOL = "read_page_draft";
export const PAGE_PATCH_TOOL = "patch_page_draft";
export const MAX_PAGE_DRAFT_SIZE = 256 * 1024;

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
