import { fireEvent, render, screen } from "@testing-library/react";
import { AgentAvatarField, avatarFileError } from "@/components/agents/agent-avatar-field";
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

jest.mock("react-easy-crop", () => ({
  __esModule: true,
  default: ({ image }: { image: string }) => <div data-testid="cropper" data-image={image} />,
}));

const mockUpload = {
  mutate: jest.fn(),
  mutateAsync: jest.fn(),
  reset: jest.fn(),
  isPending: false,
  error: null,
};
const mockSelect = { mutateAsync: jest.fn(), reset: jest.fn(), isPending: false, error: null };
const mockRemove = { mutate: jest.fn(), isPending: false, error: null };
jest.mock("@/hooks", () => ({
  useAvatarPresets: () => ({ catalog: { data: [], isLoading: false }, current: {} }),
  useAgentAvatar: () => ({ upload: mockUpload, remove: mockRemove, selectPreset: mockSelect }),
}));

describe("avatar field", () => {
  const agent = { id: "agent_1", avatar: avatar } as unknown as import("@/lib/api/types").Agent;

  beforeAll(() => {
    URL.createObjectURL = jest.fn(() => "blob:avatar");
    URL.revokeObjectURL = jest.fn();
  });

  it("validates type and size before cropping", () => {
    expect(avatarFileError(new File(["x"], "a.svg", { type: "image/svg+xml" }))).toMatch(/PNG/);
    const big = new File([new Uint8Array(10 * 1024 * 1024 + 1)], "a.png", { type: "image/png" });
    expect(avatarFileError(big)).toMatch(/10 MB/);
    expect(avatarFileError(new File(["x"], "a.png", { type: "image/png" }))).toBeNull();
  });

  it("opens the crop dialog for a dropped image and rejects other files", () => {
    render(<AgentAvatarField agent={agent} readOnly={false} />);
    const zone = screen.getByRole("button", { name: "Replace avatar" });

    fireEvent.drop(zone, {
      dataTransfer: { files: [new File(["x"], "a.txt", { type: "text/plain" })] },
    });
    expect(screen.getByText("Choose a PNG, JPEG, GIF or WebP image.")).toBeInTheDocument();
    expect(screen.queryByTestId("cropper")).toBeNull();

    fireEvent.drop(zone, {
      dataTransfer: { files: [new File(["x"], "a.png", { type: "image/png" })] },
    });
    expect(screen.getByText("Crop avatar")).toBeInTheDocument();
    expect(screen.getByTestId("cropper")).toHaveAttribute("data-image", "blob:avatar");
    expect(screen.getByRole("slider", { name: "Zoom" })).toBeInTheDocument();
  });

  it("is inert when read-only", () => {
    render(<AgentAvatarField agent={agent} readOnly />);
    expect(screen.getByRole("button", { name: "Replace avatar" })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Remove/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Choose preset" })).toBeDisabled();
  });
});
