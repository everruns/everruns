// "Fork into chat" is the one request the session recording can issue
// (EVE-854). It must hit POST /v1/sessions/{id}/fork and let the server inherit
// everything the caller does not override — agent, model, goal, tags, locale —
// so the fork carries the parent's context rather than a UI-guessed subset.

import { api } from "@/lib/api/client";
import { forkSession } from "@/lib/api/sessions";

jest.mock("@/lib/api/client", () => ({
  api: { post: jest.fn(), get: jest.fn(), patch: jest.fn(), put: jest.fn(), delete: jest.fn() },
  throwApiError: jest.fn(),
}));

const mockApi = jest.mocked(api);

describe("forkSession", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockApi.post.mockResolvedValue({ data: { id: "session_fork" } } as never);
  });

  it("posts to the session's fork endpoint", async () => {
    await expect(forkSession("session_1")).resolves.toEqual({ id: "session_fork" });

    expect(mockApi.post).toHaveBeenCalledWith("/v1/sessions/session_1/fork", {});
  });

  it("sends only the overrides it is given", async () => {
    await forkSession("session_1", { title: "Recorded run (chat)" });

    expect(mockApi.post).toHaveBeenCalledWith("/v1/sessions/session_1/fork", {
      title: "Recorded run (chat)",
    });
  });
});
