import { renderHook } from "@testing-library/react";
import { landingTarget, useLandingRedirect, withoutOpenChat } from "@/components/chat-mode/landing";

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
  it("keeps Chat as the landing route and opens it beside Home in the Chat workspace", () => {
    expect(landingTarget(false)).toBe("/chats");
    expect(landingTarget(true)).toBe("/home?chat=open");
  });

  it("waits for organization flags before choosing", () => {
    const { rerender } = renderHook(({ ready }) => useLandingRedirect(true, ready), {
      initialProps: { ready: false },
    });
    expect(replace).not.toHaveBeenCalled();
    rerender({ ready: true });
    expect(replace).toHaveBeenCalledWith("/home?chat=open");
  });

  it("only redirects an arrival, not an ordinary visit to Chat", () => {
    search = new URLSearchParams();
    renderHook(() => useLandingRedirect(true, true));
    expect(replace).not.toHaveBeenCalled();
  });

  it("drops the open-chat marker and keeps other query values", () => {
    expect(withoutOpenChat("/home", new URLSearchParams("chat=open&x=1"))).toBe("/home?x=1");
    expect(withoutOpenChat("/home", new URLSearchParams("chat=open"))).toBe("/home");
  });
});
