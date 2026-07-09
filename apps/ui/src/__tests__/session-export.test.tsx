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
});
