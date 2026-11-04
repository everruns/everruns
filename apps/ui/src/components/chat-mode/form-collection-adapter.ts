import { assertDraftSize, type PageDraftAdapter, type PageDraftSnapshot } from "./page-draft";

export interface FormCollectionEntry {
  key: string;
  label: string;
  identity: string;
  adapter: PageDraftAdapter;
}
export const formCollectionIdentity = (entries: FormCollectionEntry[]) =>
  JSON.stringify(entries.map((entry) => [entry.key, entry.identity]));

export function createFormCollectionAdapter({
  entries: getEntries,
  identity,
}: {
  entries: () => FormCollectionEntry[];
  identity: string;
}): PageDraftAdapter {
  const binding = crypto.randomUUID();
  let signature = "";
  let revision = "";
  function describe() {
    const entries = getEntries();
    if (formCollectionIdentity(entries) !== identity)
      return { binding, revision, dirty: false, writable: false };
    const descriptors = entries.map((entry) => entry.adapter.describe());
    const next = JSON.stringify(descriptors);
    if (signature !== next) {
      signature = next;
      revision = crypto.randomUUID();
    }
    return {
      binding,
      revision,
      dirty: descriptors.some((item) => item.dirty),
      writable: descriptors.some(
        (item, index) => item.writable && !!entries[index].adapter.previewPatch,
      ),
    };
  }
  function read(token: string): PageDraftSnapshot {
    if (token !== binding)
      throw new Error("The page's forms changed. Send a new message from the current page.");
    const entries = getEntries();
    if (formCollectionIdentity(entries) !== identity)
      throw new Error("The page's forms changed. Send a new message from the current page.");
    const descriptor = describe();
    const forms = entries.map((entry) => ({
      ...entry,
      snapshot: entry.adapter.read(entry.adapter.describe().binding),
    }));
    const result = {
      ...descriptor,
      entity_id: "page",
      entity_type: "page",
      values: Object.fromEntries(forms.map((form) => [form.key, form.snapshot.values])),
      forms: Object.fromEntries(
        forms.map((form) => [
          form.key,
          {
            label: form.label,
            writable: form.snapshot.writable && !!form.adapter.previewPatch,
            dirty: form.snapshot.dirty,
          },
        ]),
      ),
      patch_schema: {
        type: "object",
        minProperties: 1,
        maxProperties: 1,
        additionalProperties: false,
        properties: Object.fromEntries(
          forms.map((form) => [
            form.key,
            {
              ...form.snapshot.patch_schema,
              readOnly: !form.snapshot.writable || !form.adapter.previewPatch,
            },
          ]),
        ),
      },
    };
    assertDraftSize(result);
    return result;
  }
  return {
    binding,
    describe,
    read,
    patch: (token, expectedRevision, input) => {
      const before = read(token);
      if (before.revision !== expectedRevision)
        throw new Error("A form changed since you read the page. Read it again before editing.");
      if (
        !input ||
        typeof input !== "object" ||
        Array.isArray(input) ||
        Object.keys(input).length !== 1
      )
        throw new Error("Change one named form per patch. No changes were applied.");
      const key = Object.keys(input)[0];
      const target = getEntries().find((entry) => entry.key === key);
      if (!target?.adapter.previewPatch)
        throw new Error("This form is not available for coordinated edits.");
      const descriptor = target.adapter.describe();
      const changes = (input as Record<string, unknown>)[key];
      const preview = target.adapter.previewPatch(descriptor.binding, descriptor.revision, changes);
      // Size-check the validated aggregate before calling any React setter.
      assertDraftSize({ ...before, values: { ...before.values, [key]: preview.values } });
      target.adapter.patch(descriptor.binding, descriptor.revision, changes);
      return read(token);
    },
  };
}
