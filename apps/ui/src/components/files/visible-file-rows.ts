import type { FileInfo } from "@/lib/api/types";

// Follow the rendered tree, not every folder ever loaded. Keep one extra row so
// the page registry can report truncation without walking an unbounded catalog.
export function visibleFileRows(
  root: string,
  loaded: ReadonlyMap<string, readonly FileInfo[]>,
  expanded: ReadonlySet<string>,
) {
  const rows: Array<
    Pick<FileInfo, "id" | "path" | "name" | "is_directory" | "is_readonly" | "size_bytes">
  > = [];
  const visited = new Set<string>();
  function visit(path: string) {
    if (visited.has(path) || !expanded.has(path)) return;
    visited.add(path);
    for (const file of loaded.get(path) ?? []) {
      if (rows.length >= 51) break;
      rows.push({
        id: file.id,
        path: file.path,
        name: file.name,
        is_directory: file.is_directory,
        is_readonly: file.is_readonly,
        size_bytes: file.size_bytes,
      });
      if (file.is_directory) visit(file.path);
    }
  }
  visit(root);
  return rows;
}
