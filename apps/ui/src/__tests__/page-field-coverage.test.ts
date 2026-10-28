import { z } from "zod";
import * as schemas from "@/components/chat-mode/page-form-schemas";
import {
  createFormDraftController,
  type FormDraftState,
} from "@/components/chat-mode/form-draft-controller";

// Independent field fixtures make additions/removals visible in review. Every
// declared field exercises the real patch controller and authorization checks.
const fixtures: Record<string, [Record<string, unknown>, Record<string, unknown>]> = {
  virtualUserCreateDraftSchema: [
    { usage: "service", name: "", description: "", locale: "en", timezone: "UTC" },
    {
      usage: "end_user",
      name: "Northstar",
      description: "Help",
      locale: "uk",
      timezone: "Europe/Kyiv",
    },
  ],
  harnessCreateDraftSchema: [
    {
      display_name: "",
      name: "",
      description: "",
      system_prompt: "",
      parent_harness_id: "",
      default_model_id: "",
      tags: "",
      capabilities: [],
      initial_files: [],
      network_access: {},
    },
    {
      display_name: "Northstar",
      name: "support",
      description: "Help",
      system_prompt: "Answer support requests",
      parent_harness_id: "harness_parent",
      default_model_id: "model_muse",
      tags: "support",
      capabilities: [{ ref: "current_time", config: {} }],
      initial_files: [{ path: "/help.txt", content: "Guide", encoding: "text", is_readonly: true }],
      network_access: { allowed: ["example.test"] },
    },
  ],
  evalCreateDraftSchema: [
    {
      name: "",
      description: "",
      model_override: "",
      target_type: "session",
      harness_id: "",
      agent_id: "",
      app_id: "",
      system_prompt: "",
      model_id: "",
      tags: [],
    },
    {
      name: "Support quality",
      description: "Review",
      model_override: "model_muse",
      target_type: "app",
      harness_id: "harness_support",
      agent_id: "agent_support",
      app_id: "app_support",
      system_prompt: "Help",
      model_id: "model_default",
      tags: ["support"],
    },
  ],
  evalCaseDraftSchema: [
    { name: "", description: "", messages: "", scorer_type: "contains", scorer_value: "" },
    {
      name: "Refunds",
      description: "Review",
      messages: '[{"role":"user","content":"Help"}]',
      scorer_type: "regex",
      scorer_value: "refund",
    },
  ],
  scheduleCreateDraftSchema: [
    {
      name: "",
      description: "",
      cron_expression: "",
      target_type: "workflow",
      target_name: "",
      enabled: false,
    },
    {
      name: "Daily",
      description: "Review",
      cron_expression: "0 0 9 * * * *",
      target_type: "activity",
      target_name: "review",
      enabled: true,
    },
  ],
  scheduleEditDraftSchema: [
    {
      description: "",
      cron_expression: "",
      max_concurrent: "",
      catch_up_missed: false,
      max_catch_up: "",
    },
    {
      description: "Review",
      cron_expression: "0 0 9 * * * *",
      max_concurrent: "2",
      catch_up_missed: true,
      max_catch_up: "3",
    },
  ],
  collectionDraftSchema: [
    { name: "", description: "", mode: "off", model: "", indexer: "", rules: "{}" },
    {
      name: "Northstar",
      description: "Help",
      mode: "hybrid",
      model: "model_muse",
      indexer: "indexer_support",
      rules: '{"index_exclude":["drafts/**"]}',
    },
  ],
  sourceDraftSchema: [
    { kind: "github", coordinates: "", branch: "main", folder: "", root: "sources", identity: "" },
    {
      kind: "git",
      coordinates: "https://example.test/docs.git",
      branch: "support",
      folder: "docs",
      root: "help",
      identity: "identity_service",
    },
  ],
  providerTraceDraftSchema: [
    { enabled: false, generation_url_template: "", session_url_template: "" },
    {
      enabled: true,
      generation_url_template: "https://example.test/generation?id={response_id}",
      session_url_template: "https://example.test/session?id={session_id}",
    },
  ],
  providerAdvancedDraftSchema: [{ cache_diagnostics: false }, { cache_diagnostics: true }],
  providerConnectDraftSchema: [
    { name: "", custom_endpoint: false, base_url: "" },
    { name: "OpenRouter", custom_endpoint: true, base_url: "https://openrouter.ai/api/v1" },
  ],
  mcpMetadataDraftSchema: [
    {
      name: "",
      description: "",
      url: "",
      protocol_mode: "auto",
      elicitation_policy: "url",
      tool_timeout: "",
    },
    {
      name: "Tools",
      description: "Help",
      url: "https://example.test/mcp",
      protocol_mode: "2026-07-28",
      elicitation_policy: "url_and_form",
      tool_timeout: "60",
    },
  ],
  declarativeDraftSchema: [
    { name: "", displayName: "", description: "", skills: [], files: [] },
    {
      name: "support",
      displayName: "Northstar",
      description: "Help",
      skills: [
        {
          id: "s1",
          name: "help",
          description: "Help",
          instructions: "Review",
          files: [{ id: "f1", path: "/guide.txt", content: "Guide" }],
          userInvocable: true,
          disableModelInvocation: false,
        },
      ],
      files: [{ id: "f2", path: "/help.txt", content: "Help", access: "readonly" }],
    },
  ],
};
beforeAll(() => {
  global.structuredClone = (v) => JSON.parse(JSON.stringify(v));
});

it("has independent field fixtures for every shared schema", () => {
  expect(Object.keys(fixtures).sort()).toEqual(
    Object.keys(schemas)
      .filter((name) => name.endsWith("DraftSchema"))
      .sort(),
  );
});

function setup(name: string, writable = true) {
  const [before, after] = fixtures[name];
  const schema = (schemas as Record<string, unknown>)[name] as z.ZodType<Record<string, unknown>>;
  const state: FormDraftState<Record<string, unknown>> = {
    values: structuredClone(before),
    baseline: "v1",
    writable,
    dirty: false,
    apply: jest.fn((values) => {
      state.values = values;
      state.dirty = true;
    }),
  };
  const adapter = createFormDraftController({
    binding: "owned-form",
    entityId: "fixture",
    entityType: name,
    schema,
    state: () => state,
    changed: () => {},
  });
  return { before, after, state, adapter, schema };
}

const fields = Object.entries(fixtures).flatMap(([name, [, values]]) =>
  Object.keys(values).map((field) => ({ name, field })),
);
it.each(fields)(
  "stages $name.$field atomically and preserves every omitted field",
  ({ name, field }) => {
    const { adapter, state, before, after, schema } = setup(name);
    expect(Object.keys(z.toJSONSchema(schema).properties ?? {}).sort()).toEqual(
      Object.keys(before).sort(),
    );
    expect(after[field]).not.toEqual(before[field]);
    const read = adapter.read("owned-form");
    adapter.patch(read.binding, read.revision, { [field]: after[field] });
    expect(state.values).toEqual({ ...before, [field]: after[field] });
    expect(state.apply).toHaveBeenCalledTimes(1);
  },
);

it.each(Object.keys(fixtures))(
  "enforces read-only, revoked permissions, stale baselines and unknown fields for %s",
  (name) => {
    const { adapter, state, before, after } = setup(name, false);
    const field = Object.keys(after)[0];
    let read = adapter.read("owned-form");
    expect(() => adapter.patch(read.binding, read.revision, { [field]: after[field] })).toThrow(
      /read.only/,
    );
    state.writable = true;
    read = adapter.read("owned-form");
    expect(() =>
      adapter.patch(read.binding, read.revision, {
        [field]: after[field],
        api_key: "fixture-secret",
      }),
    ).toThrow(/unsupported/);
    expect(state.values).toEqual(before);
    state.baseline = "concurrent-save";
    expect(() => adapter.patch(read.binding, read.revision, { [field]: after[field] })).toThrow(
      /draft changed/,
    );
    read = adapter.read("owned-form");
    state.writable = false;
    expect(() => adapter.patch(read.binding, read.revision, { [field]: after[field] })).toThrow(
      /read.only/,
    );
    state.available = false;
    expect(() => adapter.read(read.binding)).toThrow(/no longer available/);
    expect(state.apply).not.toHaveBeenCalled();
  },
);
