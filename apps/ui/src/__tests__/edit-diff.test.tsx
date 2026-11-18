import { fireEvent, render, screen } from "@testing-library/react";
import { countDiffLines, normalizeEditPatch } from "@/components/chat/edit-diff";
import { WriteFileToolCallCard } from "@/components/chat/write-file-tool-call-card";
import type { ToolCompletedData } from "@/lib/api/types";

// The real view needs the browser-only diff renderer; the card's lazy import is stubbed.
jest.mock("next/dynamic", () => ({
  __esModule: true,
  default: () =>
    function EditDiffViewStub({ diff }: { diff: string }) {
      return <div data-testid="edit-diff-view">{diff}</div>;
    },
}));

const PATH = "/workspace/src/a.ts";
// Shape of render_unified_diff in crates/integrations/src/filesystem/mod.rs.
const DIFF = [
  `--- ${PATH} (before)`,
  `+++ ${PATH} (after)`,
  "@@ -1,4 +1,4 @@",
  " const a = 1;",
  "-const b = 2;",
  "-// --- old marker",
  "+const b = 3;",
  " const c = 4;",
  "",
].join("\n");

function result(payload: unknown, success = true): ToolCompletedData {
  return {
    tool_call_id: "t1",
    tool_name: "edit_file",
    success,
    status: success ? "success" : "error",
    error: success ? undefined : "old_text not found",
    result: [{ type: "text", text: JSON.stringify(payload) }],
    duration_ms: 80,
  } as unknown as ToolCompletedData;
}

const call = { id: "t1", name: "edit_file", arguments: { path: PATH } };

describe("edit diff helpers", () => {
  test("strips the before/after labels so the language comes from the extension", () => {
    const patch = normalizeEditPatch(DIFF);
    expect(patch).toContain(`--- ${PATH}\n+++ ${PATH}\n`);
    expect(patch).not.toContain("(before)");
  });

  test("drops the server's truncation marker", () => {
    const truncated = `${DIFF}\n... diff truncated after 20000 characters ...`;
    expect(normalizeEditPatch(truncated)).not.toContain("diff truncated");
  });

  test("counts added and removed lines, not file headers", () => {
    // "-// --- old marker" is a removed line, not a header.
    expect(countDiffLines(DIFF)).toEqual({ added: 1, removed: 2 });
  });

  test("an empty diff counts nothing", () => {
    expect(countDiffLines("")).toEqual({ added: 0, removed: 0 });
  });
});

describe("WriteFileToolCallCard with an edit diff", () => {
  const payload = {
    path: PATH,
    applied_edits: 1,
    first_changed_line: 2,
    diff: DIFF,
  };

  test("collapsed line leads with +/− counts", () => {
    render(<WriteFileToolCallCard toolCall={call} toolResult={result(payload)} />);
    expect(screen.getByText("+1")).toBeInTheDocument();
    expect(screen.getByText("−2")).toBeInTheDocument();
    expect(screen.queryByTestId("edit-diff-view")).not.toBeInTheDocument();
  });

  test("expanding shows the diff view instead of raw text", () => {
    render(<WriteFileToolCallCard toolCall={call} toolResult={result(payload)} />);
    fireEvent.click(screen.getByRole("button", { name: /a\.ts/ }));
    expect(screen.getByTestId("edit-diff-view")).toHaveTextContent("const b = 3;");
    expect(document.querySelector("pre")).toBeNull();
  });

  test("results without a diff keep the plain text fallback and no counts", () => {
    render(
      <WriteFileToolCallCard
        toolCall={{ ...call, name: "write_file" }}
        toolResult={result("wrote 12 bytes")}
      />,
    );
    expect(screen.queryByText(/^\+\d+$/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /a\.ts/ }));
    expect(screen.queryByTestId("edit-diff-view")).not.toBeInTheDocument();
    expect(document.querySelector("pre")).toHaveTextContent("wrote 12 bytes");
  });

  test("failed edits show the error, not a diff", () => {
    render(<WriteFileToolCallCard toolCall={call} toolResult={result(payload, false)} />);
    expect(screen.queryByText("+1")).not.toBeInTheDocument();
    expect(screen.queryByTestId("edit-diff-view")).not.toBeInTheDocument();
  });
});
