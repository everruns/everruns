import { visibleFileRows } from "@/components/files/visible-file-rows";
import type { FileInfo } from "@/lib/api/types";

function file(path: string, directory = false): FileInfo {
  return {
    id: path,
    path,
    name: path.split("/").pop()!,
    is_directory: directory,
    is_readonly: false,
    size_bytes: 0,
    ...{ content: "fixture-secret", metadata: { token: "fixture-secret" } },
  } as unknown as FileInfo;
}
it("follows visible ancestors and never exposes a formerly expanded hidden folder or file bodies", () => {
  const loaded = new Map([
    ["/", [file("/docs", true), file("/welcome.md")]],
    ["/docs", [file("/docs/nested", true)]],
    ["/docs/nested", [file("/docs/nested/hidden.md")]],
  ]);
  const collapsed = visibleFileRows("/", loaded, new Set(["/", "/docs/nested"]));
  expect(collapsed.map((row) => row.path)).toEqual(["/docs", "/welcome.md"]);
  const expanded = visibleFileRows("/", loaded, new Set(["/", "/docs", "/docs/nested"]));
  expect(expanded.map((row) => row.path)).toEqual([
    "/docs",
    "/docs/nested",
    "/docs/nested/hidden.md",
    "/welcome.md",
  ]);
  expect(JSON.stringify(expanded)).not.toContain("fixture-secret");
});
it("bounds large/cyclic catalogs before constructing context rows", () => {
  const rows = Array.from({ length: 10000 }, (_, i) => file(`/file-${i}`));
  expect(visibleFileRows("/", new Map([["/", rows]]), new Set(["/"]))).toHaveLength(51);
  expect(visibleFileRows("/", new Map([["/", [file("/", true)]]]), new Set(["/"]))).toHaveLength(1);
});
