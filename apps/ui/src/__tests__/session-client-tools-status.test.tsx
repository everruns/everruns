import { renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import {
  SessionProvider,
  useSessionContext,
} from "@/app/(main)/sessions/[sessionId]/session-context";
import type { Event, Session } from "@/lib/api/types";

let mockEvents: Event[] = [];
const mockSession: Session = {
  id: "session_chat",
  organization_id: "org_1",
  harness_id: "harness_1",
  agent_id: null,
  owner_principal_id: "user_1",
  title: null,
  tags: [],
  model_id: null,
  status: "active",
  created_at: "2026-10-19T12:00:00Z",
  updated_at: "2026-10-19T12:00:00Z",
  started_at: null,
  finished_at: null,
};
jest.mock("@/hooks", () => ({
  useSession: () => ({ data: mockSession }),
  useAgent: () => ({}),
  useModel: () => ({}),
  useSessionResolvedModel: () => ({}),
  useEvents: () => ({ data: mockEvents }),
}));
jest.mock("next/navigation", () => ({ usePathname: () => "/agents/agent_1" }));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_1" } }),
}));
jest.mock("@/providers/locale-provider", () => ({ useLocale: () => ({ locale: "en" }) }));
jest.mock("@/providers/webmcp-context", () => ({ useWebMcp: () => ({}) }));
jest.mock("@/hooks/use-webmcp-tool", () => ({ useWebMcpTool: jest.fn() }));
jest.mock("@/hooks/use-chat-sends", () => ({ useChatSends: () => ({}) }));
const mockQueryClient = { invalidateQueries: jest.fn() };
jest.mock("@tanstack/react-query", () => ({
  useMutation: () => ({ isPending: false }),
  useQueryClient: () => mockQueryClient,
}));
const ev = (type: string, sequence: number) =>
  ({
    id: `event_${sequence}`,
    type,
    sequence,
    context: {},
    data: {},
    ts: "2026-10-19T12:00:00Z",
  }) as Event;
const wrapper = ({ children }: { children: ReactNode }) => (
  <SessionProvider sessionId="session_chat">{children}</SessionProvider>
);

beforeEach(() => {
  mockEvents = [];
  mockSession.status = "active";
});

it("pauses for a live client request and resumes on the next reason step", () => {
  mockEvents = [ev("session.activated", 1)];
  const { result, rerender } = renderHook(() => useSessionContext(), { wrapper });
  expect(result.current.effectiveStatus).toBe("active");
  mockEvents = [...mockEvents, ev("tool.call_requested", 2)];
  rerender();
  expect(result.current.effectiveStatus).toBe("waiting_for_tool_results");
  expect(result.current.isActive).toBe(true);
  mockEvents = [...mockEvents, ev("tool.completed", 3), ev("reason.started", 4)];
  rerender();
  expect(result.current.effectiveStatus).toBe("active");
  mockEvents = [...mockEvents, ev("session.idled", 5)];
  rerender();
  expect(result.current.effectiveStatus).toBe("idle");
});

it("normalizes the current API waiting status before any events arrive", () => {
  mockSession.status = "waitingfortoolresults" as Session["status"];
  const { result } = renderHook(() => useSessionContext(), { wrapper });
  expect(result.current.effectiveStatus).toBe("waiting_for_tool_results");
  expect(result.current.isActive).toBe(true);
});
