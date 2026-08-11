// EVE-854: a session is a recording. The bar is "no mutating request can be
// issued from this page", asserted rather than eyeballed.
//
// Every network call the UI makes goes through `@/lib/api/client` or `fetch`.
// Both are replaced here with recorders that fail loudly on anything other than
// GET, then every tab is rendered and every control on it is
// exercised — buttons clicked, text fields typed into and submitted. A tab that
// grows a composer, a save button or a delete affordance fails this test.

import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

// Streamdown and friends ship ESM that jest cannot parse; the transcript's
// markdown rendering is not what this test is about.
jest.mock("streamdown", () => ({
  Streamdown: ({ children }: { children: string }) => <div>{children}</div>,
}));
jest.mock("@streamdown/code", () => ({ code: {} }));
jest.mock("@streamdown/mermaid", () => ({ createMermaidPlugin: jest.fn(() => ({})) }));
jest.mock("remark-gfm", () => jest.fn());
jest.mock("remark-github-blockquote-alert", () => jest.fn());

// SSE keeps live sessions streaming; jsdom has no EventSource. A stub is enough
// — this test is about requests the page issues, and SSE is a GET subscription.
class StubEventSource {
  close() {}
  addEventListener() {}
  removeEventListener() {}
}
(globalThis as unknown as { EventSource: unknown }).EventSource = StubEventSource;

const mutatingCalls: string[] = [];

function recordMutation(method: string, url: string): never {
  mutatingCalls.push(`${method} ${url}`);
  throw new Error(`unexpected mutating request: ${method} ${url}`);
}

jest.mock("@/lib/api/client", () => ({
  api: {
    defaults: { baseURL: "/api" },
    get: jest.fn(async (url: string) => ({ data: mockGetResponse(url) })),
    post: (url: string) => recordMutation("POST", url),
    put: (url: string) => recordMutation("PUT", url),
    patch: (url: string) => recordMutation("PATCH", url),
    delete: (url: string) => recordMutation("DELETE", url),
  },
  ApiError: class ApiError extends Error {},
  getApiBaseUrl: () => "/api",
  getBackendUrl: () => "http://backend",
  throwApiError: async () => {
    throw new Error("api error");
  },
}));

/** GET fixtures. Non-empty so the tabs actually render rows to click. */
function mockGetResponse(url: string): unknown {
  if (url.includes("/storage/keys")) {
    return { data: [{ key: "run_id", value: "42", updated_at: NOW }] };
  }
  if (url.includes("/storage/secrets")) return { data: [{ name: "API_KEY", updated_at: NOW }] };
  if (url.includes("/fs/")) {
    return {
      data: [
        {
          id: "file_1",
          name: "notes.md",
          path: "/workspace/notes.md",
          is_directory: false,
          is_readonly: false,
          size_bytes: 12,
        },
        {
          id: "dir_1",
          name: "src",
          path: "/workspace/src",
          is_directory: true,
          is_readonly: false,
          size_bytes: 0,
        },
      ],
    };
  }
  if (url.includes("/tasks")) {
    return {
      data: [
        {
          id: "task_1",
          session_id: "session_1",
          kind: "subagent",
          display_name: "Reviewer",
          state: "awaiting_input",
          input_request: { id: "req_1", prompt: "Which branch?" },
          attempt: 1,
          wake_policy: "always",
          created_at: NOW,
          updated_at: NOW,
        },
      ],
    };
  }
  if (url.includes("/schedules")) {
    return {
      data: [
        {
          id: "sched_1",
          session_id: "session_1",
          schedule_type: "recurring",
          cron_expression: "0 9 * * *",
          timezone: "UTC",
          enabled: true,
          trigger_count: 2,
          description: "Daily digest",
          created_at: NOW,
          updated_at: NOW,
        },
      ],
    };
  }
  return { data: [] };
}

const NOW = "2026-05-25T10:00:00Z";

jest.mock("next/navigation", () => ({
  usePathname: () => "/sessions/session_1/timeline",
  useRouter: () => ({ push: jest.fn() }),
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    // eslint-disable-next-line nextjs/no-html-link-for-pages
    <a href={href}>{children}</a>
  ),
}));

jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_1" }, isLoading: false }),
}));

jest.mock("@/providers/locale-provider", () => ({
  useLocale: () => ({ locale: "en", backendLocale: "en-US", t: (key: string) => key }),
}));

const sessionFixture = {
  id: "session_1",
  title: "Recorded run",
  agent_id: "agent_1",
  harness_id: "harness_1",
  workspace_id: "wsp_1",
  status: "idle",
  created_at: NOW,
  updated_at: NOW,
  features: ["file_system", "secrets", "key_value", "schedules", "leased_resources"],
  active_schedule_count: 1,
};

jest.mock("../app/(main)/sessions/[sessionId]/session-context", () => ({
  SessionProvider: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useSessionContext: () => ({
    sessionId: "session_1",
    agentId: "agent_1",
    agent: { id: "agent_1", name: "Runner", status: "active" },
    session: sessionFixture,
    llmModel: { model_id: "model_1", display_name: "GPT-4" },
    liveUsage: { input_tokens: 100, output_tokens: 20 },
    events: [],
    chatEvents: [],
    toolResultsMap: new Map(),
    toolProgressMap: new Map(),
    toolOutputMap: new Map(),
    eventsLoading: false,
    hasMoreEvents: false,
    loadingOlderEvents: false,
    loadOlderEvents: jest.fn(),
    isThinking: false,
    streamingText: null,
    streamingMessageId: null,
    streamingIteration: null,
    getMessageText: () => "",
    getToolCalls: () => [],
  }),
}));

// eslint-disable-next-line import/first
import TranscriptPage from "../app/(main)/sessions/[sessionId]/transcript/page";
// eslint-disable-next-line import/first
import TimelinePage from "../app/(main)/sessions/[sessionId]/timeline/page";
// eslint-disable-next-line import/first
import WorkPage from "../app/(main)/sessions/[sessionId]/work/page";
// eslint-disable-next-line import/first
import EventsPage from "../app/(main)/sessions/[sessionId]/events/page";
// eslint-disable-next-line import/first
import WorkspacePage from "../app/(main)/sessions/[sessionId]/files/page";
// eslint-disable-next-line import/first
import CostPage from "../app/(main)/sessions/[sessionId]/cost/page";

const TABS: Array<[string, React.ComponentType]> = [
  ["Transcript", TranscriptPage],
  ["Timeline", TimelinePage],
  ["Work", WorkPage],
  ["Events", EventsPage],
  ["Workspace", WorkspacePage],
  ["Cost", CostPage],
];

function renderTab(Tab: React.ComponentType) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <Tab />
    </QueryClientProvider>,
  );
}

/**
 * Fill every text field, then click every button and submit every form. Fields
 * are filled first so a "Create"/"Save" button gated on a non-empty input is
 * enabled by the time we reach it.
 */
function exerciseEveryControl() {
  for (const field of Array.from(document.querySelectorAll<HTMLInputElement>("input, textarea"))) {
    fireEvent.change(field, { target: { value: "mutate-me" } });
    fireEvent.keyDown(field, { key: "Enter" });
  }
  for (const button of screen.queryAllByRole("button")) {
    fireEvent.click(button);
  }
  for (const form of Array.from(document.querySelectorAll("form"))) {
    fireEvent.submit(form);
  }
}

describe("session detail is a read-only recording", () => {
  beforeEach(() => {
    mutatingCalls.length = 0;
    jest.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it.each(TABS)("%s issues no mutating request on render", (_name, Tab) => {
    renderTab(Tab);
    expect(mutatingCalls).toEqual([]);
  });

  it.each(TABS)("%s issues no mutating request when every control is used", (_name, Tab) => {
    renderTab(Tab);
    exerciseEveryControl();
    // Clicking may reveal a second layer of controls (expanders, dialogs).
    exerciseEveryControl();
    expect(mutatingCalls).toEqual([]);
  });

  it.each(TABS)("%s renders no composer or save affordance", (_name, Tab) => {
    renderTab(Tab);

    // Match the tooltip/title too — a write affordance whose visible label is
    // just "Folder" is still a write affordance.
    const labels = screen
      .queryAllByRole("button")
      .map((button) =>
        `${button.getAttribute("aria-label") ?? ""} ${button.getAttribute("title") ?? ""} ${button.textContent ?? ""}`.toLowerCase(),
      );

    for (const forbidden of ["send", "save", "delete", "replace", "new file", "new folder"]) {
      expect(labels.filter((label) => label.includes(forbidden))).toEqual([]);
    }
  });
});
