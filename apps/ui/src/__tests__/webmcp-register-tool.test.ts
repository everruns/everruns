import { registerWebMcpTool } from "@/lib/webmcp/register-tool";
import type { WebMcpModelContext, WebMcpToolDefinition } from "@/lib/webmcp/types";

const tool: WebMcpToolDefinition = {
  name: "everruns_test",
  description: "Test tool",
  execute: () => ({ ok: true }),
};

function documentWithModelContext(modelContext?: WebMcpModelContext): Document {
  return { modelContext } as Document;
}

describe("registerWebMcpTool", () => {
  it("is a no-op when the browser does not support WebMCP", async () => {
    const registered = await registerWebMcpTool(
      tool,
      new AbortController().signal,
      documentWithModelContext(),
    );
    expect(registered).toBe(false);
  });

  it("registers with the supplied lifecycle signal", async () => {
    const registerTool = jest.fn().mockResolvedValue(undefined);
    const modelContext = Object.assign(new EventTarget(), { registerTool });
    const controller = new AbortController();

    const registered = await registerWebMcpTool(
      tool,
      controller.signal,
      documentWithModelContext(modelContext),
    );

    expect(registered).toBe(true);
    expect(registerTool).toHaveBeenCalledWith(tool, { signal: controller.signal });
  });
});
