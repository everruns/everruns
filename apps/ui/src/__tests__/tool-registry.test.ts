import { getToolCategory } from "@/lib/tool-registry";

describe("tool registry family audit", () => {
  it.each([
    ["list_directory", "read"],
    ["stat_file", "read"],
    ["discover", "search"],
    ["query", "search"],
    ["web_fetch", "search"],
    ["tool_search", "search"],
    ["write_skill", "write"],
    ["create_schedule", "write"],
    ["execute", "tool"],
    ["spawn_background", "tool"],
    ["mcp_provider__search", "search"],
  ] as const)("classifies %s as %s", (tool, category) => {
    expect(getToolCategory(tool)).toBe(category);
  });
});
