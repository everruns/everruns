/**
 * Session export: format selection on SessionCard and the ATIF limit alerts
 * (413 size cap -> error toast, X-Atif-Images-Omitted -> info toast) in the
 * shared download flow. The default JSONL path must stay unchanged.
 */

import * as React from "react";
import { render, screen, fireEvent } from "@testing-library/react";
import { SessionCard } from "@/components/session/session-card";
import { downloadSessionExport } from "@/lib/session-export";
import type { Session } from "@/lib/api/types";

// Mock next/link - using span with data-href to avoid linting issues
jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    <span data-testid="session-link" data-href={href}>
      {children}
    </span>
  ),
}));

// base-ui menus do not open in jsdom; render all parts as passthrough elements
// (same approach as dropdown-menu.test.tsx) so menu items are clickable.
jest.mock("@base-ui/react/menu", () => {
  const ReactActual = jest.requireActual<typeof import("react")>("react");

  type PassthroughProps = React.HTMLAttributes<HTMLElement> & {
    children?: React.ReactNode;
    sideOffset?: number;
    align?: string;
  };

  const divPassthrough = ReactActual.forwardRef<HTMLElement, PassthroughProps>(
    function DivPassthrough({ children, sideOffset, align, ...props }, ref) {
      return (
        <div
          ref={ref as React.Ref<HTMLDivElement>}
          data-side-offset={sideOffset}
          data-align={align}
          {...props}
        >
          {children}
        </div>
      );
    },
  );

  const buttonPassthrough = ReactActual.forwardRef<
    HTMLButtonElement,
    React.ButtonHTMLAttributes<HTMLButtonElement>
  >(function ButtonPassthrough({ children, ...props }, ref) {
    return (
      <button ref={ref} type="button" {...props}>
        {children}
      </button>
    );
  });

  return {
    Menu: {
      Root: divPassthrough,
      Portal: divPassthrough,
      Trigger: buttonPassthrough,
      Positioner: divPassthrough,
      Popup: divPassthrough,
      Group: divPassthrough,
      Item: divPassthrough,
      CheckboxItem: divPassthrough,
      CheckboxItemIndicator: divPassthrough,
      RadioGroup: divPassthrough,
      RadioItem: divPassthrough,
      RadioItemIndicator: divPassthrough,
      GroupLabel: divPassthrough,
      Separator: divPassthrough,
      SubmenuRoot: divPassthrough,
      SubmenuTrigger: buttonPassthrough,
    },
  };
});

const SESSION_ID = "session_0d9f3a91b24e48d6a0e91f3b7c4d2e85";

const mockSession: Session = {
  id: SESSION_ID,
  organization_id: "org_default",
  harness_id: "harness-default",
  agent_id: "agent-1",
  owner_principal_id: "principal-1",
  title: "Test Session",
  tags: [],
  model_id: null,
  status: "idle",
  created_at: "2025-01-01T00:00:00Z",
  updated_at: "2025-01-01T00:00:02Z",
  started_at: "2025-01-01T00:00:01Z",
  finished_at: null,
};

describe("SessionCard export menu", () => {
  it("offers JSONL and ATIF export options", () => {
    const onExport = jest.fn();
    render(<SessionCard session={mockSession} onExport={onExport} />);

    expect(screen.getByLabelText("Export session")).toBeInTheDocument();

    fireEvent.click(screen.getByText("Export JSONL"));
    expect(onExport).toHaveBeenCalledWith(SESSION_ID, "jsonl");

    fireEvent.click(screen.getByText("Export ATIF"));
    expect(onExport).toHaveBeenCalledWith(SESSION_ID, "atif");
  });

  it("does not render the export menu without onExport", () => {
    render(<SessionCard session={mockSession} />);
    expect(screen.queryByLabelText("Export session")).not.toBeInTheDocument();
  });
});

type MockResponseOptions = {
  ok?: boolean;
  status?: number;
  statusText?: string;
  headers?: Record<string, string>;
  body?: string;
};

function mockResponse({
  ok = true,
  status = 200,
  statusText = "OK",
  headers = {},
  body = "",
}: MockResponseOptions = {}) {
  return {
    ok,
    status,
    statusText,
    headers: { get: (name: string) => headers[name] ?? null },
    blob: async () => new Blob([body]),
    text: async () => body,
  } as unknown as Response;
}

// A standalone ATIF segment document. When `cursor` is provided, it carries a
// root `continued_trajectory_ref` pointing at the next segment (mirroring the
// server contract); the final segment omits it.
function atifSegment(cursor?: string): string {
  const doc: Record<string, unknown> = { atif_version: "1.7", messages: [] };
  if (cursor) {
    doc.continued_trajectory_ref = `/v1/sessions/${SESSION_ID}/export?format=atif&segmented=true&cursor=${cursor}`;
  }
  return JSON.stringify(doc);
}

// A 413 whose RFC-9457 detail advertises segmentation (new server).
function tooLargeSegmentedResponse() {
  return mockResponse({
    ok: false,
    status: 413,
    statusText: "Payload Too Large",
    body: JSON.stringify({
      title: "Payload Too Large",
      status: 413,
      code: "atif_export_too_large",
      detail:
        "ATIF document is over the 10485760-byte limit; use &segmented=true to download it in parts",
    }),
  });
}

describe("downloadSessionExport", () => {
  const fetchMock = jest.fn();
  let downloads: string[] = [];
  let consoleErrorSpy: jest.SpyInstance;

  beforeEach(() => {
    downloads = [];
    fetchMock.mockReset();
    global.fetch = fetchMock as unknown as typeof fetch;
    URL.createObjectURL = jest.fn(() => "blob:mock");
    URL.revokeObjectURL = jest.fn();
    jest
      .spyOn(HTMLAnchorElement.prototype, "click")
      .mockImplementation(function (this: HTMLAnchorElement) {
        downloads.push(this.download);
      });
    consoleErrorSpy = jest.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it("downloads JSONL by default without any toast", async () => {
    fetchMock.mockResolvedValueOnce(mockResponse());
    const notify = jest.fn();

    await downloadSessionExport(SESSION_ID, "jsonl", "en", notify);

    expect(fetchMock).toHaveBeenCalledWith(`/api/v1/sessions/${SESSION_ID}/export`, {
      credentials: "include",
    });
    expect(downloads).toEqual([`${SESSION_ID}.jsonl`]);
    expect(notify).not.toHaveBeenCalled();
  });

  it("downloads ATIF via ?format=atif as {sessionId}.atif.json", async () => {
    fetchMock.mockResolvedValueOnce(mockResponse());
    const notify = jest.fn();

    await downloadSessionExport(SESSION_ID, "atif", "en", notify);

    expect(fetchMock).toHaveBeenCalledWith(`/api/v1/sessions/${SESSION_ID}/export?format=atif`, {
      credentials: "include",
    });
    expect(downloads).toEqual([`${SESSION_ID}.atif.json`]);
    expect(notify).not.toHaveBeenCalled();
  });

  it("shows the server message as an error toast on 413", async () => {
    fetchMock.mockResolvedValueOnce(
      mockResponse({
        ok: false,
        status: 413,
        statusText: "Payload Too Large",
        body: JSON.stringify({ detail: "ATIF document exceeds the 10 MiB export cap" }),
      }),
    );
    const notify = jest.fn();

    await downloadSessionExport(SESSION_ID, "atif", "en", notify);

    expect(downloads).toEqual([]);
    expect(notify).toHaveBeenCalledWith({
      title: "ATIF export failed",
      body: "ATIF document exceeds the 10 MiB export cap",
    });
  });

  it("falls back to a clear message when the 413 body is not parseable", async () => {
    fetchMock.mockResolvedValueOnce(
      mockResponse({ ok: false, status: 413, statusText: "Payload Too Large" }),
    );
    const notify = jest.fn();

    await downloadSessionExport(SESSION_ID, "atif", "en", notify);

    expect(downloads).toEqual([]);
    expect(notify).toHaveBeenCalledWith({
      title: "ATIF export failed",
      body: "This session is too large for ATIF export.",
    });
  });

  it("still downloads and shows an info toast when images were omitted", async () => {
    fetchMock.mockResolvedValueOnce(mockResponse({ headers: { "X-Atif-Images-Omitted": "3" } }));
    const notify = jest.fn();

    await downloadSessionExport(SESSION_ID, "atif", "en", notify);

    expect(downloads).toEqual([`${SESSION_ID}.atif.json`]);
    expect(notify).toHaveBeenCalledWith({
      title: "ATIF export",
      body: "3 images were omitted from the ATIF export",
    });
  });

  it("uses the singular form for one omitted image", async () => {
    fetchMock.mockResolvedValueOnce(mockResponse({ headers: { "X-Atif-Images-Omitted": "1" } }));
    const notify = jest.fn();

    await downloadSessionExport(SESSION_ID, "atif", "en", notify);

    expect(notify).toHaveBeenCalledWith({
      title: "ATIF export",
      body: "1 image was omitted from the ATIF export",
    });
  });

  it("keeps the legacy console-only behavior for non-413 errors", async () => {
    fetchMock.mockResolvedValueOnce(
      mockResponse({ ok: false, status: 500, statusText: "Internal Server Error" }),
    );
    const notify = jest.fn();

    await downloadSessionExport(SESSION_ID, "atif", "en", notify);

    expect(downloads).toEqual([]);
    expect(notify).not.toHaveBeenCalled();
    expect(consoleErrorSpy).toHaveBeenCalled();
  });

  it("offers a segmented download on a segmentation-capable 413 and walks all parts", async () => {
    fetchMock
      .mockResolvedValueOnce(tooLargeSegmentedResponse())
      .mockResolvedValueOnce(mockResponse({ body: atifSegment("c1") }))
      .mockResolvedValueOnce(mockResponse({ body: atifSegment("c2") }))
      .mockResolvedValueOnce(mockResponse({ body: atifSegment() }));
    const notify = jest.fn();

    await downloadSessionExport(SESSION_ID, "atif", "en", notify);

    // The offer is a single toast with an action; nothing downloads yet.
    expect(downloads).toEqual([]);
    expect(notify).toHaveBeenCalledTimes(1);
    const offer = notify.mock.calls[0][0];
    expect(offer.title).toBe("ATIF export");
    expect(offer.body).toBe(
      "This session is too large for a single ATIF file. Download it in parts instead?",
    );
    expect(offer.action.label).toBe("Download in parts");

    // Confirm → the segmented walk runs.
    await offer.action.onClick();

    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      `/api/v1/sessions/${SESSION_ID}/export?format=atif&segmented=true`,
      { credentials: "include" },
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      3,
      `/api/v1/sessions/${SESSION_ID}/export?format=atif&segmented=true&cursor=c1`,
      { credentials: "include" },
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      4,
      `/api/v1/sessions/${SESSION_ID}/export?format=atif&segmented=true&cursor=c2`,
      { credentials: "include" },
    );
    expect(downloads).toEqual([
      `${SESSION_ID}.atif.part1.json`,
      `${SESSION_ID}.atif.part2.json`,
      `${SESSION_ID}.atif.part3.json`,
    ]);
    expect(notify).toHaveBeenLastCalledWith({
      title: "ATIF export",
      body: "Exported 3 ATIF parts",
    });
  });

  it("aggregates omitted images across segments into one info toast", async () => {
    fetchMock
      .mockResolvedValueOnce(tooLargeSegmentedResponse())
      .mockResolvedValueOnce(
        mockResponse({ headers: { "X-Atif-Images-Omitted": "2" }, body: atifSegment("c1") }),
      )
      .mockResolvedValueOnce(
        mockResponse({ headers: { "X-Atif-Images-Omitted": "3" }, body: atifSegment() }),
      );
    const notify = jest.fn();

    await downloadSessionExport(SESSION_ID, "atif", "en", notify);
    await notify.mock.calls[0][0].action.onClick();

    expect(downloads).toEqual([`${SESSION_ID}.atif.part1.json`, `${SESSION_ID}.atif.part2.json`]);
    expect(notify).toHaveBeenCalledWith({ title: "ATIF export", body: "Exported 2 ATIF parts" });
    expect(notify).toHaveBeenLastCalledWith({
      title: "ATIF export",
      body: "5 images were omitted from the ATIF export",
    });
  });

  it("stops with an error toast when a mid-walk segment returns a bad cursor (400)", async () => {
    fetchMock
      .mockResolvedValueOnce(tooLargeSegmentedResponse())
      .mockResolvedValueOnce(mockResponse({ body: atifSegment("c1") }))
      .mockResolvedValueOnce(
        mockResponse({
          ok: false,
          status: 400,
          statusText: "Bad Request",
          body: JSON.stringify({ code: "atif_cursor_invalid", detail: "invalid cursor" }),
        }),
      );
    const notify = jest.fn();

    await downloadSessionExport(SESSION_ID, "atif", "en", notify);
    await notify.mock.calls[0][0].action.onClick();

    // First part saved, then the walk halts on the bad cursor.
    expect(downloads).toEqual([`${SESSION_ID}.atif.part1.json`]);
    expect(notify).toHaveBeenLastCalledWith({
      title: "ATIF export failed",
      body: "ATIF export stopped after 1 part",
    });
  });

  it("falls back to a plain error toast when a 413 does not advertise segmentation", async () => {
    // Old server: same too-large `code`, but the detail does not mention
    // segmentation, so no "Download in parts" offer is made.
    fetchMock.mockResolvedValueOnce(
      mockResponse({
        ok: false,
        status: 413,
        statusText: "Payload Too Large",
        body: JSON.stringify({
          code: "atif_export_too_large",
          detail: "ATIF document exceeds the 10 MiB export cap",
        }),
      }),
    );
    const notify = jest.fn();

    await downloadSessionExport(SESSION_ID, "atif", "en", notify);

    expect(downloads).toEqual([]);
    expect(notify).toHaveBeenCalledTimes(1);
    expect(notify).toHaveBeenCalledWith({
      title: "ATIF export failed",
      body: "ATIF document exceeds the 10 MiB export cap",
    });
  });
});
