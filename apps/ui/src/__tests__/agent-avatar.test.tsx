import { render } from "@testing-library/react";
import { AgentAvatar } from "@/components/agents/agent-avatar";
import { agentAvatarUrl } from "@/lib/api/agents";
import type { AgentAvatar as AgentAvatarData } from "@/lib/api/agent-types";

const avatar: AgentAvatarData = {
  id: "avatar_01933b5a000070008000000000000001",
  url: "/v1/avatars/avatar_01933b5a000070008000000000000001/square-256.png",
  circle_url: "/v1/avatars/avatar_01933b5a000070008000000000000001/circle-256.png",
  sizes: [32, 64, 128, 256, 512],
};

describe("agent avatar", () => {
  it("picks the smallest preset that covers the requested size", () => {
    expect(agentAvatarUrl(avatar, 64)).toBe(`/api/v1/avatars/${avatar.id}/square-64.png`);
    expect(agentAvatarUrl(avatar, 76, "circle")).toBe(
      `/api/v1/avatars/${avatar.id}/circle-128.png`,
    );
    expect(agentAvatarUrl(avatar, 2000)).toBe(`/api/v1/avatars/${avatar.id}/square-512.png`);
  });

  it("renders a 2x preset, or the fallback when there is no avatar", () => {
    const { container, rerender } = render(
      <AgentAvatar avatar={avatar} size={28} fallback={<span data-testid="fallback" />} />,
    );
    const img = container.querySelector("img");
    expect(img?.getAttribute("src")).toBe(`/api/v1/avatars/${avatar.id}/square-64.png`);
    expect(img?.getAttribute("width")).toBe("28");

    rerender(<AgentAvatar avatar={null} size={28} fallback={<span data-testid="fallback" />} />);
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("[data-testid=fallback]")).not.toBeNull();
  });
});
