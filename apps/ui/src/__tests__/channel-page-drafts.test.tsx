import { useState } from "react";
import { act, render } from "@testing-library/react";
import { PageChatProvider } from "@/test-utils/page-chat-provider";
import { useChannelChatDraft } from "@/components/chat-mode/use-channel-chat-draft";
import {
  type ChannelFormState,
  getDefaultChannelFormState,
} from "@/components/agents/channels/channel-form";
import type { ChannelType } from "@/lib/api/types";
import type { ChatModeState } from "@/components/chat-mode/chat-mode-context";
import type { PageDraftAdapter } from "@/components/chat-mode/page-draft";
let adapter: PageDraftAdapter;
const mode: ChatModeState = {
  page: { route: "/agents/agent_1/channels/new", label: "Channel" },
  pageContainer: { current: null },
  open: true,
  setOpen: () => {},
  actions: [],
  recordAction: () => {},
  registerPage: (_, value) => {
    adapter = value!;
    return () => {};
  },
};
function Form({ kind }: { kind: ChannelType }) {
  const [state, setState] = useState<ChannelFormState>(() => ({
    ...getDefaultChannelFormState(kind),
    slackSigningSecret: "fixture-signing-secret",
    publicChatToken: "fixture-token",
    api: {
      ...getDefaultChannelFormState(kind).api,
      authMethods: [{ api_key: "fixture-api-secret" }],
    },
  }));
  useChannelChatDraft({ state, setState, baseline: "new", writable: true });
  return null;
}
beforeAll(() => {
  global.structuredClone = (v) => JSON.parse(JSON.stringify(v));
});
it.each<ChannelType>([
  "public_chat",
  "schedule",
  "webhook",
  "ag_ui",
  "fcp",
  "slack",
  "voice",
  "api",
  "work_chat",
])("stages %s channel metadata and excludes credentials and auth configuration", (kind) => {
  render(
    <PageChatProvider value={mode}>
      <Form kind={kind} />
    </PageChatProvider>,
  );
  const before = adapter.read(adapter.describe().binding);
  expect(JSON.stringify(before)).not.toContain("fixture-");
  expect(() =>
    adapter.patch(before.binding, before.revision, { slackSigningSecret: "overwrite" }),
  ).toThrow(/unsupported/);
  if (kind === "api" || kind === "work_chat") {
    expect(() =>
      adapter.patch(before.binding, before.revision, {
        api: { ...(before.values?.api as object), authMethods: [] },
      }),
    ).toThrow();
  }
  act(() => {
    adapter.patch(before.binding, before.revision, { enabled: false });
  });
  expect(adapter.read(before.binding).values).toMatchObject({ enabled: false });
});
it("accepts every existing Slack response and session choice and Voice interruption mode", () => {
  const { unmount } = render(
    <PageChatProvider value={mode}>
      <Form kind="slack" />
    </PageChatProvider>,
  );
  const slack = adapter.read(adapter.describe().binding);
  act(() => {
    adapter.patch(slack.binding, slack.revision, {
      slackSessionStrategy: "per_user",
      slackResponsePolicy: "relevant_messages",
    });
  });
  expect(adapter.read(slack.binding).values).toMatchObject({ slackSessionStrategy: "per_user" });
  unmount();
  render(
    <PageChatProvider value={mode}>
      <Form kind="voice" />
    </PageChatProvider>,
  );
  const voice = adapter.read(adapter.describe().binding);
  act(() => {
    adapter.patch(voice.binding, voice.revision, {
      voice: { ...(voice.values?.voice as object), interruption: "cancel" },
    });
  });
  expect(adapter.read(voice.binding).values).toMatchObject({ voice: { interruption: "cancel" } });
});
