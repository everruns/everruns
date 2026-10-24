import {
  canEditObserverScorers,
  observerDraftSchema,
  observerReadScorersSchema,
} from "@/components/chat-mode/observer-draft-schema";
import { createFormDraftController } from "@/components/chat-mode/form-draft-controller";

beforeAll(() => {
  global.structuredClone = (value) => JSON.parse(JSON.stringify(value));
});
const fields = {
  name: "Review",
  description: "",
  sampling_rate: 0.1,
  agent_ids: [],
  harness_ids: [],
  session_tags: "",
};

it.each([
  { type: "citation_faithful", min_citations: 1, pass_threshold: 0.8 },
  { type: "citation_judged", pass_threshold: 0.8 },
  { type: "json_schema", schema: true },
])("preserves API scorer variants that the editor cannot create: $type", (rule) => {
  const values = observerReadScorersSchema.parse({
    ...fields,
    scorers: [{ key: "quality", scope: "turn", method: "rule", rule }],
  });
  expect(canEditObserverScorers(values.scorers)).toBe(false);
  const state = {
    values,
    baseline: "v1",
    dirty: false,
    writable: true,
    readonlyFields: ["scorers"],
    apply: jest.fn(),
  };
  state.apply.mockImplementation((next) => {
    state.values = next;
    state.dirty = true;
  });
  const adapter = createFormDraftController({
    binding: "observer",
    entityId: "observer_1",
    entityType: "observer",
    schema: observerReadScorersSchema,
    state: () => state,
    changed: () => {},
  });
  const before = adapter.read("observer");
  expect(before.patch_schema?.properties).toMatchObject({ scorers: { readOnly: true } });
  const after = adapter.patch(before.binding, before.revision, { name: "Reviewed metadata" });
  expect(after.values.scorers).toEqual(values.scorers);
  expect(() => adapter.patch(after.binding, after.revision, { scorers: [] })).toThrow(
    /read-only field/,
  );
  expect(state.apply).toHaveBeenCalledTimes(1);
});

it("only stages scopes supported by the server and rejects unknown scorer fields", () => {
  const scorer = {
    key: "quality",
    scope: "turn",
    method: "llm_judge",
    rubric: "Be accurate.",
    pass_threshold: 0.5,
  };
  expect(canEditObserverScorers([scorer])).toBe(true);
  expect(observerDraftSchema.safeParse({ ...fields, scorers: [scorer] }).success).toBe(true);
  expect(
    observerDraftSchema.safeParse({ ...fields, scorers: [{ ...scorer, scope: "session" }] })
      .success,
  ).toBe(false);
  expect(
    observerReadScorersSchema.safeParse({
      ...fields,
      scorers: [{ ...scorer, api_key: "fixture-secret" }],
    }).success,
  ).toBe(false);
});
