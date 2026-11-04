import { z } from "zod";
import { assertDraftSize, type PageDraftAdapter, type PageDraftSnapshot } from "./page-draft";

export interface FormDraftState<T extends Record<string, unknown>> {
  values: T;
  dirty: boolean;
  writable: boolean;
  available?: boolean;
  readonlyFields?: readonly string[];
  baseline: string;
  apply: (values: T) => void;
}

// Form schemas are explicit allowlists of non-credential fields. A controller
// only stages React state; it has no save callback, API client or DOM access.
export function createFormDraftController<T extends Record<string, unknown>>({
  binding,
  entityId,
  entityType,
  schema,
  state,
  changed,
}: {
  binding: string;
  entityId: string;
  entityType: string;
  schema: z.ZodType<T>;
  state: () => FormDraftState<T>;
  changed: (before: PageDraftSnapshot<T>, after: PageDraftSnapshot<T>, fields: string[]) => void;
}): PageDraftAdapter<T> & { settled: () => void } {
  let optimistic: PageDraftSnapshot<T> | undefined;
  let signature = "";
  let revision = "";
  const patchSchema = z.toJSONSchema(schema, { unrepresentable: "any" }) as Record<string, unknown>;
  if (patchSchema.additionalProperties !== false)
    throw new Error("Page drafts require an explicit field allowlist.");
  delete patchSchema.required;
  function track(current: FormDraftState<T>) {
    const values = Object.fromEntries(
      Object.keys(current.values)
        .sort()
        .map((key) => [key, current.values[key]]),
    );
    const nextSignature = JSON.stringify([
      entityId,
      current.baseline,
      values,
      current.writable,
      current.readonlyFields,
    ]);
    if (nextSignature !== signature) {
      signature = nextSignature;
      revision = crypto.randomUUID();
    }
  }
  function snapshot(): PageDraftSnapshot<T> {
    const current = state();
    if (current.available === false)
      throw new Error(
        "This form is no longer available. Send a new message from the current page.",
      );
    if (optimistic) return { ...optimistic, writable: current.writable };
    assertDraftSize(current.values);
    schema.parse(current.values);
    const values = current.values;
    track(current);
    const properties = patchSchema.properties as Record<string, Record<string, unknown>>;
    const editableSchema = {
      ...patchSchema,
      properties: Object.fromEntries(
        Object.entries(properties).map(([key, definition]) => [
          key,
          current.readonlyFields?.includes(key) ? { ...definition, readOnly: true } : definition,
        ]),
      ),
    };
    return {
      binding,
      revision,
      entity_id: entityId,
      entity_type: entityType,
      values,
      dirty: current.dirty,
      writable: current.writable,
      patch_schema: editableSchema,
    };
  }
  function read(token: string) {
    if (token !== binding)
      throw new Error(
        "This form is no longer the current page. Send a new message from that page.",
      );
    const value = snapshot();
    assertDraftSize(value);
    return structuredClone(value);
  }
  function prepare(token: string, expectedRevision: string, input: unknown) {
    const before = read(token);
    if (!before.writable)
      throw new Error("This form is read-only, loading, or saving. No changes were applied.");
    if (before.revision !== expectedRevision)
      throw new Error(
        "The draft changed since you read it. Read it again and preserve the user's edits.",
      );
    assertDraftSize(input);
    if (!input || typeof input !== "object" || Array.isArray(input))
      throw new Error("Provide an object of supported form fields.");
    const fields = Object.keys(input);
    if (
      !fields.length ||
      fields.length > 64 ||
      fields.some((field) => !Object.hasOwn(before.values, field))
    )
      throw new Error("The patch includes an unsupported form field. No changes were applied.");
    if (fields.some((field) => state().readonlyFields?.includes(field)))
      throw new Error("The patch includes a read-only field. No changes were applied.");
    // Validate the complete candidate before invoking any setter. Save keeps
    // its own business validation; draft schemas allow incomplete form text.
    const parsed = schema.parse({ ...before.values, ...input });
    const values = {
      ...before.values,
      ...Object.fromEntries(fields.map((field) => [field, parsed[field]])),
    };
    const modified = fields.filter(
      (field) => JSON.stringify(before.values[field]) !== JSON.stringify(values[field]),
    );
    if (!modified.length) return { before, after: before, modified };
    const after = { ...before, values, revision: crypto.randomUUID(), dirty: true };
    assertDraftSize(after);
    return { before, after, modified };
  }
  return {
    binding,
    describe: () => {
      if (state().available === false) return { binding, revision, dirty: false, writable: false };
      if (optimistic)
        return {
          binding,
          revision: optimistic.revision,
          dirty: optimistic.dirty,
          writable: optimistic.writable,
        };
      const current = state();
      track(current);
      return { binding, revision, dirty: current.dirty, writable: current.writable };
    },
    read,
    settled: () => {
      optimistic = undefined;
    },
    previewPatch: (token, expectedRevision, input) =>
      structuredClone(prepare(token, expectedRevision, input).after),
    patch: (token, expectedRevision, input) => {
      const { before, after, modified } = prepare(token, expectedRevision, input);
      if (!modified.length) return before;
      optimistic = after;
      const ordered = Object.fromEntries(
        Object.keys(after.values)
          .sort()
          .map((key) => [key, after.values[key]]),
      );
      signature = JSON.stringify([
        entityId,
        state().baseline,
        ordered,
        state().writable,
        state().readonlyFields,
      ]);
      revision = after.revision;
      try {
        state().apply(after.values);
      } catch (error) {
        optimistic = undefined;
        throw error;
      }
      changed(before, after, modified);
      return structuredClone(after);
    },
  };
}
