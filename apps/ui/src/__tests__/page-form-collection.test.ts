import { z } from "zod";
import { createFormDraftController } from "@/components/chat-mode/form-draft-controller";
import {
  createFormCollectionAdapter,
  formCollectionIdentity,
} from "@/components/chat-mode/form-collection-adapter";

beforeAll(() => {
  global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
});
function fixture() {
  const states = ["Details", "Tracing"].map((name) => ({
    values: { name },
    baseline: "v1",
    dirty: false,
    writable: true,
    apply: jest.fn(),
  }));
  const entries = states.map((state, index) => {
    state.apply.mockImplementation((values) => {
      state.values = values;
      state.dirty = true;
    });
    const adapter = createFormDraftController({
      binding: `form-${index}`,
      entityId: "provider",
      entityType: "provider",
      schema: z.strictObject({ name: z.string() }),
      state: () => state,
      changed: () => {},
    });
    return {
      key: ["details", "tracing"][index],
      label: state.values.name,
      identity: adapter.binding!,
      adapter,
    };
  });
  let active = entries;
  const adapter = createFormCollectionAdapter({
    entries: () => active,
    identity: formCollectionIdentity(entries),
  });
  return {
    states,
    entries,
    adapter,
    replace: () => {
      active = entries.slice(0, 1);
    },
  };
}

it("names independent Save forms and edits one without invoking another", () => {
  const { adapter, states } = fixture();
  const before = adapter.read(adapter.binding!);
  expect(Object.keys(before.forms!)).toEqual(["details", "tracing"]);
  const after = adapter.patch(before.binding, before.revision, { tracing: { name: "New trace" } });
  expect(states[0].apply).not.toHaveBeenCalled();
  expect(states[1].apply).toHaveBeenCalledTimes(1);
  expect(after.values).toEqual({ details: { name: "Details" }, tracing: { name: "New trace" } });
});

it("rejects mixed, unknown and stale form edits before any setter", () => {
  const { adapter, states } = fixture();
  const before = adapter.read(adapter.binding!);
  expect(() =>
    adapter.patch(before.binding, before.revision, {
      details: { name: "A" },
      tracing: { name: "B" },
    }),
  ).toThrow(/one named/);
  expect(() => adapter.patch(before.binding, before.revision, { api_keys: { name: "A" } })).toThrow(
    /not available/,
  );
  states[0].values.name = "Manual draft";
  expect(() => adapter.patch(before.binding, before.revision, { tracing: { name: "B" } })).toThrow(
    /changed/,
  );
  expect(states.every((state) => state.apply.mock.calls.length === 0)).toBe(true);
});

it("size-checks the complete result before applying a valid child patch", () => {
  const { adapter, states } = fixture();
  states[1].values.name = "b".repeat(80 * 1024);
  const before = adapter.read(adapter.binding!);
  expect(() =>
    adapter.patch(before.binding, before.revision, { details: { name: "a".repeat(200 * 1024) } }),
  ).toThrow(/large|limit|size/i);
  expect(states[0].apply).not.toHaveBeenCalled();
});

it("invalidates a preceding collection when a dialog/form set changes", () => {
  const { adapter, replace } = fixture();
  const before = adapter.read(adapter.binding!);
  replace();
  expect(adapter.describe().writable).toBe(false);
  expect(() => adapter.read(before.binding)).toThrow(/forms changed/);
  expect(() =>
    adapter.patch(before.binding, before.revision, { details: { name: "Old request" } }),
  ).toThrow(/forms changed/);
});
