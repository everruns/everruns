import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { UrlElicitationToolCall } from "@/components/chat/url-elicitation-tool-call";
import { buildToolActivityGroups } from "@/components/chat/tool-activity-groups";
import type { Event, ToolCompletedData } from "@/lib/api/types";

const submitElicitationConsent = jest.fn();

jest.mock("@/lib/api/sessions", () => ({
  submitElicitationConsent: (...args: unknown[]) => submitElicitationConsent(...args),
}));

const elicitation = {
  server: "Example Billing",
  tool: "charge",
  message: "Authorize the payment to continue.",
  url: "https://pay.example.com/authorize/42?token=abc",
  url_host: "pay.example.com",
  url_is_punycode: false,
};

function renderCard(overrides: Partial<typeof elicitation> = {}) {
  return render(
    <UrlElicitationToolCall
      sessionId="session_1"
      toolCallId="url_elicitation_1"
      elicitation={{ ...elicitation, ...overrides }}
      toolResultsMap={new Map<string, ToolCompletedData>()}
    />,
  );
}

beforeEach(() => {
  submitElicitationConsent.mockReset();
  submitElicitationConsent.mockResolvedValue({ host: "pay.example.com", status: "active" });
});

afterEach(() => {
  jest.restoreAllMocks();
});

describe("UrlElicitationToolCall", () => {
  it("shows the whole URL and which server is asking", () => {
    renderCard();
    // The user must be able to read what they are about to open, in full.
    expect(screen.getByText(/pay\.example\.com/)).toBeInTheDocument();
    expect(screen.getByText(/authorize\/42\?token=abc/)).toBeInTheDocument();
    expect(screen.getByText(/Example Billing/)).toBeInTheDocument();
    expect(screen.getByText(/Authorize the payment to continue\./)).toBeInTheDocument();
  });

  it("warns about a Punycode domain instead of hiding it", () => {
    renderCard({ url: "https://xn--80ak6aa92e.com/pay", url_host: "xn--80ak6aa92e.com" });
    expect(screen.queryByText(/Punycode/)).not.toBeInTheDocument();

    renderCard({
      url: "https://xn--80ak6aa92e.com/pay",
      url_host: "xn--80ak6aa92e.com",
      url_is_punycode: true,
    });
    expect(screen.getAllByText(/Punycode/).length).toBeGreaterThan(0);
  });

  it("opens nothing until the user says so, then records the consent", async () => {
    const open = jest.spyOn(window, "open").mockImplementation(() => null);
    renderCard();

    expect(open).not.toHaveBeenCalled();
    expect(submitElicitationConsent).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: /Open and continue/ }));

    expect(open).toHaveBeenCalledWith(
      "https://pay.example.com/authorize/42?token=abc",
      "_blank",
      "noopener,noreferrer",
    );
    await waitFor(() =>
      expect(submitElicitationConsent).toHaveBeenCalledWith(
        "session_1",
        "url_elicitation_1",
        "accept",
      ),
    );
  });

  it("declines without opening anything", async () => {
    const open = jest.spyOn(window, "open").mockImplementation(() => null);
    renderCard();

    fireEvent.click(screen.getByRole("button", { name: /Don't open/ }));

    await waitFor(() =>
      expect(submitElicitationConsent).toHaveBeenCalledWith(
        "session_1",
        "url_elicitation_1",
        "decline",
      ),
    );
    expect(open).not.toHaveBeenCalled();
  });

  it("leaves confirm_url_elicitation to this card rather than the activity timeline", () => {
    const request: Event = {
      id: "request",
      type: "tool.call_requested",
      ts: "2026-08-08T00:00:01Z",
      sequence: 1,
      session_id: "session_1",
      context: { turn_id: "turn-1", exec_id: "exec-1" },
      data: {
        headline: "Waiting for the user",
        tool_calls: [
          { id: "url_elicitation_1", name: "confirm_url_elicitation", arguments: elicitation },
        ],
        tool_summaries: [{ id: "url_elicitation_1", name: "confirm_url_elicitation" }],
      },
    };
    const built = buildToolActivityGroups([request], "Working");
    expect(built.byAnchorEventId.size).toBe(0);
    expect(built.groupedEventIds.has("request")).toBe(false);
  });
});
