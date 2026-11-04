import { act, renderHook } from "@testing-library/react";
import { useAgentDraft } from "@/components/agents/use-agent-draft";
import { useHarnessDraft } from "@/components/harnesses/use-harness-draft";
import { useAgentChatDraft } from "@/components/chat-mode/use-agent-chat-draft";
import { useHarnessChatDraft } from "@/components/chat-mode/use-harness-chat-draft";
import { entityLocalizationsDraftSchema } from "@/components/chat-mode/page-draft";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import type { ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { Agent, Harness } from "@/lib/api/types";

const saved = { uk: { display_name: "Підтримка", description: "Допомога" } };
const record = {
  name: "northstar",
  display_name: "Northstar",
  system_prompt: "Preserve instructions",
  harness_id: "harness_fixture",
  capabilities: [],
  initial_files: [],
  starters: [],
  tags: [],
  localizations: saved,
};
const mode: ChatModeState = {
  page: { route: "/harnesses/harness_fixture", label: "Harness" },
  pageContainer: { current: null },
  open: true,
  setOpen: () => {},
  actions: [],
  recordAction: () => {},
  registerPage: () => () => {},
};
beforeAll(() => {
  global.structuredClone = (v) => JSON.parse(JSON.stringify(v));
});

function setup(kind: "agent" | "harness") {
  return renderHook(
    ({ writable }) => {
      const agent = useAgentDraft({ ...record, id: "agent_fixture" } as unknown as Agent);
      const harness = useHarnessDraft({ ...record, id: "harness_fixture" } as unknown as Harness);
      const a = useAgentChatDraft({
        agentId: "agent_fixture",
        updatedAt: "saved",
        draft: agent,
        writable,
        blocked: false,
        beginEdit: () => {},
      });
      const h = useHarnessChatDraft({
        id: "harness_fixture",
        label: "Harness",
        updatedAt: "saved",
        draft: harness,
        writable,
        available: true,
        beginEdit: () => {},
      });
      return kind === "agent"
        ? { draft: agent, adapter: a.adapter, undo: a.changes.undo }
        : { draft: harness, adapter: h.adapter, undo: h.undo };
    },
    {
      initialProps: { writable: true },
      wrapper: ({ children }) => <PageChatProvider value={mode}>{children}</PageChatProvider>,
    },
  );
}

it.each(["agent", "harness"] as const)(
  "reads, stages, builds Save and undoes the %s localization UI without changing instructions",
  (kind) => {
    const { result, rerender } = setup(kind);
    const adapter = result.current.adapter;
    const before = adapter.read(adapter.describe().binding);
    expect(Object.keys(before.values).sort()).toEqual(
      Object.keys(before.patch_schema?.properties ?? {}).sort(),
    );
    expect(before.values.localizations).toEqual(saved);
    act(() => {
      adapter.patch(before.binding, before.revision, {
        localizations: { ...saved, "fr-ca": { display_name: "Assistance", description: "Aide" } },
      });
    });
    expect(result.current.draft.localizations["fr-ca"]).toEqual({
      display_name: "Assistance",
      description: "Aide",
    });
    expect(result.current.draft.fields.system_prompt).toBe("Preserve instructions");
    expect(result.current.draft.fields.name).toBe("northstar");
    expect(result.current.draft.buildRequest()).toMatchObject({
      ok: true,
      request: {
        localizations: { ...saved, "fr-ca": { display_name: "Assistance", description: "Aide" } },
      },
    });
    act(() => result.current.undo("localizations"));
    expect(result.current.draft.localizations).toEqual(saved);
    const latest = adapter.read(adapter.describe().binding);
    rerender({ writable: false });
    expect(() => adapter.patch(latest.binding, latest.revision, { localizations: {} })).toThrow(
      /read.only/,
    );
    expect(result.current.draft.localizations).toEqual(saved);
  },
);

it.each(["agent", "harness"] as const)(
  "rejects a mixed invalid %s localization patch atomically and protects a later manual edit",
  (kind) => {
    const { result } = setup(kind);
    const adapter = result.current.adapter;
    const before = adapter.read(adapter.describe().binding);
    expect(() =>
      adapter.patch(before.binding, before.revision, {
        system_prompt: "Partial change",
        localizations: { fr: { system_prompt: "Injected" } },
      }),
    ).toThrow();
    expect(result.current.draft.fields.system_prompt).toBe("Preserve instructions");
    act(() => {
      adapter.patch(before.binding, before.revision, {
        localizations: { fr: { display_name: "Chat" } },
      });
    });
    act(() => result.current.draft.setLocalizations({ fr: { display_name: "Manual" } }));
    act(() => result.current.undo("localizations"));
    expect(result.current.draft.localizations).toEqual({ fr: { display_name: "Manual" } });
  },
);

it.each([
  { "invalid!": {} },
  { en: { api_key: "fixture-secret" } },
  { en: { description: "x".repeat(2001) } },
  { en: { display_name: "x".repeat(256) } },
  { en: { display_name: "NUL\0" } },
  { "fr-CA": {}, fr_ca: {} },
  Object.fromEntries(Array.from({ length: 65 }, (_, i) => [`en-${i}`, {}])),
])("bounds locale tags, values and overlay authority: %j", (input) => {
  expect(entityLocalizationsDraftSchema.safeParse(input).success).toBe(false);
});
