import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { useChatThreads } from "@/hooks/use-chat-threads";
import { notifySessionChange } from "@/lib/session-activity";
import { listSessions } from "@/lib/api/sessions";
jest.mock("@/lib/api/sessions", () => ({
  listSessions: jest.fn(async () => ({ data: [], total: 0 })),
}));
jest.mock("@/hooks/use-agents", () => ({
  useAgent: () => ({ data: { id: "agent_1" }, isLoading: false }),
}));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_1" }, isLoading: false }),
}));

beforeEach(() => jest.clearAllMocks());
function setup(poll = true) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return renderHook(() => useChatThreads({ poll }), { wrapper });
}
it("coalesces observed session transitions into one list read within 250ms", async () => {
  const { result, unmount } = setup();
  await waitFor(() => expect(result.current.isRead).toBe(true));
  jest.useFakeTimers();
  try {
    act(() => {
      notifySessionChange("other_org", "session");
      jest.advanceTimersByTime(250);
    });
    expect(listSessions).toHaveBeenCalledTimes(1);
    act(() => {
      for (let i = 0; i < 100; i++) notifySessionChange("org_1", "session");
      jest.advanceTimersByTime(249);
    });
    expect(listSessions).toHaveBeenCalledTimes(1);
    await act(async () => {
      jest.advanceTimersByTime(1);
    });
    expect(listSessions).toHaveBeenCalledTimes(2);
    act(() => notifySessionChange("org_1", "session"));
    unmount();
    await act(async () => {
      jest.advanceTimersByTime(1000);
    });
    expect(listSessions).toHaveBeenCalledTimes(2);
  } finally {
    jest.useRealTimers();
  }
});
it("keeps read-once callers free of background invalidations", async () => {
  const { result, unmount } = setup(false);
  await waitFor(() => expect(result.current.isRead).toBe(true));
  jest.useFakeTimers();
  try {
    await act(async () => {
      notifySessionChange("org_1", "session");
      jest.advanceTimersByTime(1000);
    });
    expect(listSessions).toHaveBeenCalledTimes(1);
    unmount();
  } finally {
    jest.useRealTimers();
  }
});
