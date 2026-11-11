import { renderHook } from "@testing-library/react";
import { landingTarget, useLandingRedirect } from "@/components/chat-mode/landing";

const replace = jest.fn();
let search = new URLSearchParams("landing");
let pathname = "/chats";
jest.mock("next/navigation", () => ({
  usePathname: () => pathname,
  useSearchParams: () => search,
  useRouter: () => ({ replace }),
}));

beforeEach(() => {
  replace.mockClear();
  search = new URLSearchParams("landing");
  pathname = "/chats";
});

describe("landing route", () => {
  it("lands chat-first people on Overview, with Chat beside it", () => {
    expect(landingTarget("chat_first")).toBe("/home");
  });

  it("lands classic people on full-screen Chat", () => {
    expect(landingTarget("classic")).toBe("/chats");
  });

  it("waits for the organization and layout before redirecting", () => {
    const { rerender } = renderHook(({ ready }) => useLandingRedirect(ready, "chat_first"), {
      initialProps: { ready: false },
    });
    expect(replace).not.toHaveBeenCalled();
    rerender({ ready: true });
    expect(replace).toHaveBeenCalledWith("/home");
  });

  it("only redirects an arrival, not an ordinary visit to Chat", () => {
    search = new URLSearchParams();
    renderHook(() => useLandingRedirect(true, "chat_first"));
    expect(replace).not.toHaveBeenCalled();
  });
});
