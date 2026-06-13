/**
 * Regression tests for EVE-558 / EVE-555: workspace paths with URL delimiter
 * characters must be encoded before they are appended to URLs so that `?`, `#`,
 * and `%` in filenames are treated as literal bytes and do not alter the HTTP
 * method, query string, or fragment.
 */

import { listFiles, readFile, deleteFile } from "@/lib/api/session-files";
import { api } from "@/lib/api/client";

jest.mock("@/lib/api/client", () => ({
  api: {
    get: jest.fn(),
    post: jest.fn(),
    put: jest.fn(),
    delete: jest.fn(),
  },
}));

const mockedApi = api as jest.Mocked<typeof api>;

const SESSION_ID = "session_5c7f3a91b24e48d6a0e91f3b7c4d2e85";
// Derived workspace ID — same hex suffix, different prefix.
const WSP_ID = "wsp_5c7f3a91b24e48d6a0e91f3b7c4d2e85";

beforeEach(() => {
  jest.clearAllMocks();
});

describe("fsPath encoding (EVE-558 / EVE-555)", () => {
  it("encodes ? in a filename so it is not treated as a query delimiter", async () => {
    mockedApi.get.mockResolvedValueOnce({ data: { data: [] } });
    await listFiles(SESSION_ID, "/dir?recursive=true");
    expect(mockedApi.get).toHaveBeenCalledWith(
      `/v1/workspaces/${WSP_ID}/fs/dir%3Frecursive%3Dtrue`,
    );
  });

  it("encodes # in a filename so it is not treated as a fragment delimiter", async () => {
    mockedApi.get.mockResolvedValueOnce({ data: {} });
    await readFile(SESSION_ID, "/notes#section");
    expect(mockedApi.get).toHaveBeenCalledWith(`/v1/workspaces/${WSP_ID}/fs/notes%23section`);
  });

  it("encodes % in a filename so pre-encoded sequences are not double-decoded", async () => {
    mockedApi.get.mockResolvedValueOnce({ data: {} });
    await readFile(SESSION_ID, "/file%20name.txt");
    expect(mockedApi.get).toHaveBeenCalledWith(`/v1/workspaces/${WSP_ID}/fs/file%2520name.txt`);
  });

  it("preserves directory structure while encoding each segment independently", async () => {
    mockedApi.get.mockResolvedValueOnce({ data: {} });
    await readFile(SESSION_ID, "/a?b/c#d/e%f");
    expect(mockedApi.get).toHaveBeenCalledWith(`/v1/workspaces/${WSP_ID}/fs/a%3Fb/c%23d/e%25f`);
  });

  it("root path produces bare fs base URL without trailing slash", async () => {
    mockedApi.get.mockResolvedValueOnce({ data: { data: [] } });
    await listFiles(SESSION_ID, "/");
    expect(mockedApi.get).toHaveBeenCalledWith(`/v1/workspaces/${WSP_ID}/fs`);
  });

  it("recursive=true is appended as a separate query param after the encoded path", async () => {
    mockedApi.get.mockResolvedValueOnce({ data: { data: [] } });
    await listFiles(SESSION_ID, "/dir?x=1", true);
    const url = mockedApi.get.mock.calls[0][0] as string;
    // Path segment is encoded; recursive flag is a distinct query param.
    expect(url).toMatch(/\/dir%3Fx%3D1\?recursive=true$/);
  });

  it("delete with recursive=true encodes path and appends query param correctly", async () => {
    mockedApi.delete.mockResolvedValueOnce({ data: { deleted: true } });
    await deleteFile(SESSION_ID, "/tmp?dir", true);
    const url = mockedApi.delete.mock.calls[0][0] as string;
    expect(url).toBe(`/v1/workspaces/${WSP_ID}/fs/tmp%3Fdir?recursive=true`);
  });
});
