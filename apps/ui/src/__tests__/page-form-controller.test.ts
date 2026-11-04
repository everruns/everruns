import { z } from "zod";
import {
  createFormDraftController,
  type FormDraftState,
} from "@/components/chat-mode/form-draft-controller";

const schema = z.strictObject({
  name: z.string(),
  description: z.string(),
  rate: z.number().min(0).max(1),
});
beforeAll(() => {
  global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
});
function fixture() {
  const apply = jest.fn((values) => {
    state.values = values;
    state.dirty = true;
  });
  const changed = jest.fn();
  const state: FormDraftState<z.infer<typeof schema>> = {
    values: { name: "Example", description: "My manual text", rate: 0.1 },
    dirty: true,
    writable: true,
    baseline: "version-1",
    apply,
  };
  const adapter = createFormDraftController({
    binding: "browser-form",
    entityId: "observer_1",
    entityType: "observer",
    schema,
    state: () => state,
    changed,
  });
  return { adapter, state, apply, changed };
}

it("stages validated fields in the existing form without saving or overwriting omitted values", () => {
  const { adapter, state, apply } = fixture();
  const before = adapter.read("browser-form");
  const next = adapter.patch(before.binding, before.revision, { name: "Better name" });
  expect(apply).toHaveBeenCalledTimes(1);
  expect(state.values.description).toBe("My manual text");
  expect(next.values.name).toBe("Better name");
  expect(next.dirty).toBe(true);
  expect(next.revision).not.toBe(before.revision);
  expect(() => adapter.patch(before.binding, before.revision, { rate: 0.8 })).toThrow(/changed/);
});

it("validates every requested field before changing any field", () => {
  const { adapter, apply } = fixture();
  const before = adapter.read("browser-form");
  expect(() =>
    adapter.patch(before.binding, before.revision, { name: "Should not change", rate: 4 }),
  ).toThrow();
  expect(() => adapter.patch(before.binding, before.revision, { api_key: "not-shared" })).toThrow(
    /unsupported/,
  );
  expect(apply).not.toHaveBeenCalled();
  expect(adapter.read(before.binding).values).toEqual(before.values);
});

it("rejects stale reads after manual edits and permission changes", () => {
  const { adapter, state, apply } = fixture();
  const before = adapter.read("browser-form");
  state.values = { ...state.values, description: "New manual edit" };
  expect(() => adapter.patch(before.binding, before.revision, { name: "Old request" })).toThrow(
    /changed/,
  );
  state.writable = false;
  const locked = adapter.read(before.binding);
  expect(() => adapter.patch(locked.binding, locked.revision, { name: "Forbidden" })).toThrow(
    /read-only/,
  );
  expect(() => adapter.read("another-browser")).toThrow(/current page/);
  expect(apply).not.toHaveBeenCalled();
});

it("fails closed when a form accidentally includes an undeclared credential", () => {
  const { adapter, state } = fixture();
  Object.assign(state.values, { api_key: "fixture-secret" });
  expect(() => adapter.read("browser-form")).toThrow();
});

it("cannot read a replacement form through an older binding", () => {
  const { adapter, state } = fixture();
  state.available = false;
  state.values = { name: "Another entity", description: "Private replacement", rate: 0.2 };
  expect(adapter.describe().writable).toBe(false);
  expect(() => adapter.read("browser-form")).toThrow(/no longer available/);
});

it("preserves field-level restrictions while allowing other fields", () => {
  const { adapter, state, apply } = fixture();
  state.readonlyFields = ["name"];
  const before = adapter.read("browser-form");
  expect(() =>
    adapter.patch(before.binding, before.revision, {
      name: "Forbidden",
      description: "Also rejected",
    }),
  ).toThrow(/read-only field/);
  expect(apply).not.toHaveBeenCalled();
  adapter.patch(before.binding, before.revision, { description: "Allowed" });
  expect(state.values.name).toBe("Example");
  expect(state.values.description).toBe("Allowed");
});

it("keeps a reserved revision stable until React commits the staged values", () => {
  const { adapter, state, apply } = fixture();
  apply.mockImplementationOnce(() => {});
  const before = adapter.read("browser-form");
  const next = adapter.patch(before.binding, before.revision, { name: "Queued edit" });
  expect(adapter.describe().revision).toBe(next.revision);
  state.values = next.values;
  adapter.settled();
  expect(adapter.read(before.binding).revision).toBe(next.revision);
});

it("invalidates optimistic results when a form unmounts before React commits", () => {
  const { adapter, state, apply } = fixture();
  apply.mockImplementationOnce(() => {});
  const before = adapter.read("browser-form");
  const next = adapter.patch(before.binding, before.revision, { name: "Queued edit" });
  state.available = false;
  expect(() => adapter.read(next.binding)).toThrow(/no longer available/);
});
