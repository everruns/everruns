import { render, screen } from "@testing-library/react";
import { McpClientDetails } from "@/components/mcp/mcp-client-attribution";
import { mcpClientAttribution } from "@/lib/mcp-attribution";
import { MessageInfoIcon } from "@/components/chat/message-info-icon";
import type { Event } from "@/lib/api/types";

jest.mock("@/providers/locale-provider", () => ({
  useUiTranslations: () => (key: string, values?: Record<string, string>) =>
    key.replace("{name}", values?.name ?? ""),
  useLocale: () => ({ t: (key: string) => key, displayLocale: "en-US" }),
}));

test("legacy and malformed provenance stays absent", () => {
  for (const value of [undefined, null, {}, { client_id: "a" }, { client_id: 5 }]) {
    expect(mcpClientAttribution(value)).toBeUndefined();
  }
});

test("message attribution follows each input and does not label assistant output or legacy input", () => {
  const event = (type: string, name?: string) =>
    ({
      id: "event_test",
      session_id: "session_test",
      context: {},
      type,
      ts: "2026-11-04T12:00:00Z",
      metadata: name
        ? {
            mcp_client: {
              client_id: `client_${name}`,
              grant_id: `grant_${name}`,
              client_name: name,
            },
          }
        : undefined,
      data: {
        message: {
          metadata: {
            everruns_mcp_client: {
              client_id: "forged",
              grant_id: "forged",
              client_name: "Legacy Forgery",
            },
          },
        },
      },
    }) as Event;
  const { rerender } = render(<MessageInfoIcon event={event("input.message", "Client A")} />);
  expect(screen.getByText("Client A")).toBeInTheDocument();
  rerender(<MessageInfoIcon event={event("input.message", "Client B")} />);
  expect(screen.queryByText("Client A")).toBeNull();
  expect(screen.getByText("Client B")).toBeInTheDocument();
  rerender(<MessageInfoIcon event={event("output.message.completed", "Client B")} />);
  expect(screen.queryByText("Client B")).toBeNull();
  rerender(<MessageInfoIcon event={event("input.message")} />);
  expect(screen.queryByText("Client B")).toBeNull();
  expect(screen.queryByText("Legacy Forgery")).toBeNull();
});

test("shows the grant and self-declared name as text without rendering markup", () => {
  const client = mcpClientAttribution({
    client_id: "client-a",
    grant_id: "grant-a",
    client_name: "<img src=x onerror=alert(1)>",
    client_secret: "must not propagate",
  })!;
  expect(client).not.toHaveProperty("client_secret");
  const { container } = render(<McpClientDetails client={client} />);
  expect(screen.getByText(/self-declared/)).toBeInTheDocument();
  expect(screen.getByText(/client-a/)).toBeInTheDocument();
  expect(screen.getByText(/grant-a/)).toBeInTheDocument();
  expect(container.querySelector("img")).toBeNull();
});
