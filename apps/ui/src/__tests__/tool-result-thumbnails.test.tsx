import { fireEvent, render, screen, within } from "@testing-library/react";
import { ToolActivityRow } from "@/components/chat/tool-activity-row";
import { extractResultImages } from "@/components/chat/tool-call-utils";
import type { ContentPart, ToolCompletedData } from "@/lib/api/types";

const FRAME = "iVBORw0KGgo=";

describe("extractResultImages", () => {
  it("returns data URLs for raster image parts and skips everything else", () => {
    const result: ContentPart[] = [
      { type: "text", text: '{"status":"ok","action":"left_click"}' },
      { type: "image", base64: FRAME, media_type: "image/png" },
      { type: "image", base64: "PHN2Zy8+", media_type: "image/svg+xml" },
      { type: "image", url: "javascript:alert(1)" },
      { type: "image", url: "https://example.com/frame.jpg", media_type: "image/jpeg" },
    ];

    expect(extractResultImages(result)).toEqual([
      `data:image/png;base64,${FRAME}`,
      "https://example.com/frame.jpg",
    ]);
  });

  it("is empty without a result and caps a long strip", () => {
    expect(extractResultImages(undefined)).toEqual([]);
    expect(extractResultImages([{ type: "text", text: "no images" }])).toEqual([]);
    const many: ContentPart[] = Array.from({ length: 9 }, () => ({
      type: "image" as const,
      base64: FRAME,
      media_type: "image/png",
    }));
    expect(extractResultImages(many)).toHaveLength(4);
  });
});

describe("ToolActivityRow screenshots", () => {
  const toolCall = {
    id: "call-computer",
    name: "computer",
    arguments: { action: "left_click", coordinate: [120, 48] },
  };

  it("shows a computer-use screenshot as a thumbnail that opens full size", () => {
    const toolResult: ToolCompletedData = {
      tool_call_id: "call-computer",
      tool_name: "computer",
      success: true,
      status: "success",
      result: [
        { type: "text", text: '{"status":"ok","action":"left_click"}' },
        { type: "image", base64: FRAME, media_type: "image/png" },
      ],
    };

    render(
      <ToolActivityRow toolCall={toolCall} toolResult={toolResult} mode="server" locale="en" />,
    );

    const strip = screen.getByTestId("tool-result-thumbnails");
    const thumbnail = within(strip).getByAltText("Tool output image 1");
    expect(thumbnail).toHaveAttribute("src", `data:image/png;base64,${FRAME}`);

    fireEvent.click(screen.getByRole("button", { name: "Open tool output image 1 full size" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByAltText("Tool output image 1")).toHaveAttribute(
      "src",
      `data:image/png;base64,${FRAME}`,
    );
  });

  it("renders no thumbnails for text-only or unsafe image results", () => {
    const toolResult: ToolCompletedData = {
      tool_call_id: "call-computer",
      tool_name: "computer",
      success: true,
      status: "success",
      result: [
        { type: "text", text: '{"status":"ok"}' },
        { type: "image", base64: "PHN2Zy8+", media_type: "image/svg+xml" },
      ],
    };

    render(
      <ToolActivityRow toolCall={toolCall} toolResult={toolResult} mode="server" locale="en" />,
    );

    expect(screen.queryByTestId("tool-result-thumbnails")).not.toBeInTheDocument();
    expect(screen.queryByRole("img")).not.toBeInTheDocument();
  });
});
