import {
  getToolEntry,
  getToolCategory,
  isReadFileTool,
  isWriteLikeTool,
} from "@/lib/tool-registry";

describe("tool-registry", () => {
  it("classifies managed sandbox file tools consistently", () => {
    expect(getToolEntry("sandbox_read_file")).toEqual({
      category: "read",
      segmentMode: "standalone",
    });
    expect(getToolEntry("sandbox_write_file")).toEqual({
      category: "write",
      segmentMode: "standalone",
    });
  });

  it("treats sandbox_read_file as a read tool", () => {
    expect(getToolCategory("sandbox_read_file")).toBe("read");
    expect(isReadFileTool("sandbox_read_file")).toBe(true);
  });

  it("treats sandbox_write_file as a write-like tool", () => {
    expect(getToolCategory("sandbox_write_file")).toBe("write");
    expect(isWriteLikeTool("sandbox_write_file")).toBe(true);
  });
});
