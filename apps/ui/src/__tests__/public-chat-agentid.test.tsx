import { renderHook } from "@testing-library/react";

import {
  agentIdLoginUrl,
  useAgentIdSession,
} from "@/app/(public)/public-chat/[appId]/agentid-signin";

const CHANNEL = "appchan_0123456789abcdef0123456789abcdef";

describe("Public Chat AgentID sign-in", () => {
  beforeEach(() => {
    window.sessionStorage.clear();
    window.history.replaceState(null, "", "/public-chat/" + CHANNEL);
  });

  it("starts the sign-in on the channel's own login route", () => {
    expect(agentIdLoginUrl(CHANNEL)).toBe(`/api/v1/channels/${CHANNEL}/public-chat/agentid/login`);
  });

  it("moves the returned token out of the address bar into session storage", () => {
    window.history.replaceState(
      null,
      "",
      `/public-chat/${CHANNEL}#agentid_token=runtime-jwt&expires_in=900`,
    );
    const { result } = renderHook(() => useAgentIdSession(CHANNEL));
    expect(result.current.token).toBe("runtime-jwt");
    expect(window.location.hash).toBe("");
    const stored = JSON.parse(
      window.sessionStorage.getItem(`everruns_public_chat_agentid:${CHANNEL}`) ?? "{}",
    );
    expect(stored.token).toBe("runtime-jwt");
  });

  it("keeps a stored session until it expires", () => {
    window.sessionStorage.setItem(
      `everruns_public_chat_agentid:${CHANNEL}`,
      JSON.stringify({ token: "still-valid", expiresAt: Date.now() + 60_000 }),
    );
    expect(renderHook(() => useAgentIdSession(CHANNEL)).result.current.token).toBe("still-valid");

    window.sessionStorage.setItem(
      `everruns_public_chat_agentid:${CHANNEL}`,
      JSON.stringify({ token: "expired", expiresAt: Date.now() - 1 }),
    );
    expect(renderHook(() => useAgentIdSession(CHANNEL)).result.current.token).toBeNull();
  });

  it("explains a refused sign-in without exposing which check failed", () => {
    window.history.replaceState(null, "", `/public-chat/${CHANNEL}#agentid_error=owner_limit`);
    const { result } = renderHook(() => useAgentIdSession(CHANNEL));
    expect(result.current.token).toBeNull();
    expect(result.current.error).toMatch(/agent limit/);
    expect(window.location.hash).toBe("");
  });
});
