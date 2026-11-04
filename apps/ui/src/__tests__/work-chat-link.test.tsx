import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { api } from "@/lib/api/client";
import { ChannelUsePanel } from "@/components/agents/channel-use-panel";
import { WorkChatAddress, workChatUrl } from "@/components/agents/channels/work-chat-link";
import type { AgentChannel } from "@/lib/api/types";

jest.mock("@/lib/api/client", () => ({
  api: { get: jest.fn() },
}));

const get = api.get as jest.Mock;

function channel(overrides: Partial<AgentChannel> = {}): AgentChannel {
  return {
    id: "appchan_abc",
    channel_type: "work_chat",
    channel_config: {},
    enabled: true,
    status: "live",
    created_at: "2026-10-29T00:00:00Z",
    updated_at: "2026-10-29T00:00:00Z",
    ...overrides,
  };
}

function renderAddress(value: AgentChannel) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <WorkChatAddress channel={value} />
    </QueryClientProvider>,
  );
}

describe("work chat link", () => {
  beforeEach(() => {
    get.mockReset();
  });

  it("accepts only http(s) chat addresses", () => {
    expect(workChatUrl("https://abc.chat.example")).toBe("https://abc.chat.example");
    expect(workChatUrl("http://localhost:3000")).toBe("http://localhost:3000");
    expect(workChatUrl("javascript:alert(1)")).toBeNull();
    expect(workChatUrl("not a url")).toBeNull();
    expect(workChatUrl(null)).toBeNull();
  });

  it("links a published channel to its hosted chat", async () => {
    get.mockResolvedValue({ data: { public_url: "https://abc.chat.example" } });
    renderAddress(channel());

    const link = await screen.findByRole("link", { name: "https://abc.chat.example" });
    expect(link).toHaveAttribute("href", "https://abc.chat.example");
    expect(link).toHaveAttribute("target", "_blank");
    expect(screen.getByRole("button", { name: "Copy link" })).toBeInTheDocument();
    expect(get).toHaveBeenCalledWith("/v1/channels/appchan_abc/chat/config");
  });

  it("does not offer a link before the channel is published", () => {
    renderAddress(channel({ status: "draft" }));
    expect(screen.getByText(/hosted chat domain/)).toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
    expect(get).not.toHaveBeenCalled();
  });

  it("ignores an address that is not a chat URL", async () => {
    get.mockResolvedValue({ data: { public_url: "javascript:alert(1)" } });
    renderAddress(channel());
    expect(await screen.findByText(/hosted chat domain/)).toBeInTheDocument();
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
  });

  it("does not repeat the chat link on the generic connect panel", () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const { container } = render(
      <QueryClientProvider client={client}>
        <ChannelUsePanel channel={channel()} />
      </QueryClientProvider>,
    );
    expect(container).toBeEmptyDOMElement();
    expect(get).not.toHaveBeenCalled();
  });
});
