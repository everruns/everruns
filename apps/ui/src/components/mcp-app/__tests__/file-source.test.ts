import { workspaceFileSource } from "@/components/files/file-source";

it("avoids cookie-authenticated navigations and downloads through the installed fetch transport", async () => {
  const original = global.fetch;
  const blob = new Blob(["file bytes"], { type: "application/octet-stream" });
  const fetcher = jest.fn(async () => ({ ok: true, blob: async () => blob }) as Response);
  global.fetch = fetcher;
  try {
    const source = workspaceFileSource("wsp_demo", {
      readOnly: true,
      browserUrls: false,
      externalPreviewHref: "/sessions/session_demo/files",
    });
    expect(source.downloadUrl("/workspace/report%.txt")).toBeUndefined();
    expect(source.htmlPreviewUrl?.("/workspace/index.html")).toBeUndefined();
    expect(source.writes).toBeUndefined();
    expect(source.externalPreviewHref).toBe("/sessions/session_demo/files");
    await expect(source.download!("/workspace/report%.txt")).resolves.toBe(blob);
    expect(fetcher).toHaveBeenCalledWith(
      "/api/v1/workspaces/wsp_demo/fs/_/download/workspace/report%25.txt",
    );
  } finally {
    global.fetch = original;
  }
});

it("retains normal browser URLs outside an embedded transport", () => {
  const source = workspaceFileSource("wsp_demo", { readOnly: true });
  expect(source.downloadUrl("/workspace/report.txt")).toBe(
    "/api/v1/workspaces/wsp_demo/fs/_/download/workspace/report.txt",
  );
  expect(source.htmlPreviewUrl?.("/workspace/index.html")).toBe(
    "/api/v1/workspaces/wsp_demo/fs/_/preview/workspace/index.html",
  );
  expect(source.download).toBeUndefined();
});
