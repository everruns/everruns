/**
 * Centralized tool registry — single source of truth for tool name classification.
 *
 * Every tool-name magic string that was scattered across card components and
 * the activity-group dispatcher now lives here. Adding or renaming a tool
 * requires editing only this file.
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type ToolCategory = "read" | "search" | "write" | "shell" | "todo" | "tool";

export interface ToolRegistryEntry {
  /** UI category used for grouping and summarisation. */
  category: ToolCategory;
  /**
   * When `"standalone"`, the tool gets its own dedicated card in the activity
   * segment list instead of being folded into a grouped-activity card.
   */
  segmentMode: "standalone" | "grouped";
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/**
 * Static registry of known tool names.
 *
 * Tools not listed here fall through to the `"tool"` category with grouped
 * segment mode (the previous default behaviour).
 */
const TOOL_REGISTRY: Record<string, ToolRegistryEntry> = {
  // Shell tools (standalone cards)
  bash: { category: "shell", segmentMode: "standalone" },
  shell: { category: "shell", segmentMode: "standalone" },
  execute_bash: { category: "shell", segmentMode: "standalone" },

  // Read tools
  read_file: { category: "read", segmentMode: "standalone" },
  session_read_file: { category: "read", segmentMode: "standalone" },
  read_many_files: { category: "read", segmentMode: "grouped" },
  list_files: { category: "read", segmentMode: "grouped" },
  list_capabilities: { category: "read", segmentMode: "grouped" },

  // Search tools
  search: { category: "search", segmentMode: "grouped" },
  search_web: { category: "search", segmentMode: "grouped" },
  grep_files: { category: "search", segmentMode: "grouped" },

  // Write tools (standalone cards)
  write_file: { category: "write", segmentMode: "standalone" },
  edit_file: { category: "write", segmentMode: "standalone" },
  replace_in_file: { category: "write", segmentMode: "standalone" },
  append_file: { category: "write", segmentMode: "standalone" },
  move_file: { category: "write", segmentMode: "grouped" },
  delete_file: { category: "write", segmentMode: "grouped" },
  mkdir: { category: "write", segmentMode: "grouped" },

  // Todo tools
  write_todos: { category: "todo", segmentMode: "standalone" },

  // Store tools
  secret_store: { category: "tool", segmentMode: "grouped" },
  kv_store: { category: "tool", segmentMode: "grouped" },
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Look up the registry entry for a tool name. */
export function getToolEntry(toolName: string): ToolRegistryEntry | undefined {
  return TOOL_REGISTRY[toolName];
}

/** Get the category for a tool, with dynamic suffix matching for MCP search tools. */
export function getToolCategory(toolName: string): ToolCategory {
  const entry = TOOL_REGISTRY[toolName];
  if (entry) return entry.category;
  // MCP tools ending with `__search` are treated as search tools.
  if (toolName.endsWith("__search")) return "search";
  return "tool";
}

/** True when the tool name represents a bash/shell command. */
export function isBashTool(toolName: string): boolean {
  return getToolCategory(toolName) === "shell";
}

/** True when the tool name represents a read-file operation. */
export function isReadFileTool(toolName: string): boolean {
  return toolName === "read_file" || toolName === "session_read_file";
}

/** True when the tool name represents a write-like file mutation. */
export function isWriteLikeTool(toolName: string): boolean {
  return (
    toolName === "write_file" ||
    toolName === "edit_file" ||
    toolName === "replace_in_file" ||
    toolName === "append_file"
  );
}

/** True when the tool name is the write_todos planner tool. */
export function isWriteTodosTool(toolName: string): boolean {
  return toolName === "write_todos";
}

/**
 * Determine the segment mode for a tool. Standalone tools break out of the
 * grouped activity card and render their own dedicated card component.
 */
export function getSegmentMode(toolName: string): "standalone" | "grouped" {
  return TOOL_REGISTRY[toolName]?.segmentMode ?? "grouped";
}
