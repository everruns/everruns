import { fireEvent, render, screen, within } from "@testing-library/react";
import { ToolActivityRow } from "@/components/chat/tool-activity-row";
import {
  extractGeneratedGifs,
  GeneratedImageCard,
  hasGeneratedImages,
} from "@/components/chat/generated-gif-player";
import type { ToolCompletedData } from "@/lib/api/types";

const STILL = "iVBORw0KGgo=";

function gifResult(artifactId: unknown): string {
  return JSON.stringify({
    format: "gif",
    images: [
      {
        index: 1,
        artifact_id: artifactId,
        filename: "lantern.gif",
        media_type: "image/gif",
        frames: 9,
        frame_duration_ms: 160,
      },
    ],
  });
}

describe("extractGeneratedGifs", () => {
  it("reads GIF artifacts from image tool results only", () => {
    expect(extractGeneratedGifs("generate_image", gifResult("img_01"))).toEqual([
      {
        artifactId: "img_01",
        filename: "lantern.gif",
        frames: 9,
        frameDurationMs: 160,
      },
    ]);
    expect(extractGeneratedGifs("computer", gifResult("img_01"))).toEqual([]);
    expect(extractGeneratedGifs("generate_image", "not json")).toEqual([]);
  });

  it("rejects artifact ids that are not plain ids", () => {
    expect(extractGeneratedGifs("generate_image", gifResult("../../evil"))).toEqual([]);
    expect(extractGeneratedGifs("generate_image", gifResult(null))).toEqual([]);
  });
});

describe("ToolActivityRow generated GIF", () => {
  it("plays the stored GIF, labels its loop, and pauses on the first frame", () => {
    const toolResult: ToolCompletedData = {
      tool_call_id: "call-image",
      tool_name: "generate_image",
      success: true,
      status: "success",
      result: [
        { type: "text", text: gifResult("img_01") },
        { type: "image", base64: STILL, media_type: "image/png" },
      ],
    };
    render(
      <ToolActivityRow
        toolCall={{
          id: "call-image",
          name: "generate_image",
          arguments: { format: "gif" },
        }}
        toolResult={toolResult}
        mode="server"
        locale="en"
      />,
    );

    const player = screen.getByTestId("generated-gif");
    expect(screen.queryByTestId("tool-result-thumbnails")).not.toBeInTheDocument();
    expect(within(player).getByText("GIF · 9 frames · 1.4 s loop")).toBeInTheDocument();
    const image = within(player).getByAltText("lantern.gif");
    expect(image.getAttribute("src")).toMatch(/\/v1\/images\/img_01$/);

    fireEvent.click(within(player).getByRole("button", { name: /Pause/ }));
    expect(image).toHaveAttribute("src", `data:image/png;base64,${STILL}`);
    expect(within(player).getByRole("link", { name: /Download/ })).toHaveAttribute(
      "download",
      "lantern.gif",
    );
  });
});

describe("GeneratedImageCard", () => {
  it("names the model and file, and plays the GIF", () => {
    const result: ToolCompletedData = {
      tool_call_id: "call-image",
      tool_name: "generate_image",
      success: true,
      status: "success",
      duration_ms: 6400,
      result: [
        {
          type: "text",
          text: JSON.stringify({
            model: "gpt-image-2.6-flare",
            images: [
              {
                artifact_id: "img_01",
                filename: "lantern.gif",
                media_type: "image/gif",
                size_bytes: 412 * 1024,
                frames: 9,
                frame_duration_ms: 200,
              },
            ],
          }),
        },
        { type: "image", base64: STILL, media_type: "image/png" },
      ],
    };
    expect(hasGeneratedImages("generate_image", result)).toBe(true);
    expect(hasGeneratedImages("computer", result)).toBe(false);
    render(<GeneratedImageCard toolName="generate_image" result={result} />);
    const card = screen.getByTestId("generated-image-card");
    expect(within(card).getByText("gpt-image-2.6-flare · 6.4 s")).toBeInTheDocument();
    expect(within(card).getByText("lantern.gif · 412 KB")).toBeInTheDocument();
    expect(within(card).getByText("GIF · 9 frames · 1.8 s loop")).toBeInTheDocument();
  });
});
