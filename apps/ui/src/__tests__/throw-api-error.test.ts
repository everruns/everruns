import { throwApiError, ApiError } from "@/lib/api/client";

function mockResponse(
  status: number,
  statusText: string,
  body?: string,
): { status: number; statusText: string; text: () => Promise<string> } {
  return {
    status,
    statusText,
    text: () => Promise.resolve(body ?? ""),
  };
}

describe("throwApiError", () => {
  it("extracts error field from JSON body", async () => {
    const response = mockResponse(400, "Bad Request", JSON.stringify({ error: "invalid input" }));
    try {
      await throwApiError(response as unknown as Response);
      fail("should throw");
    } catch (e) {
      expect(e).toBeInstanceOf(ApiError);
      expect((e as ApiError).status).toBe(400);
      expect((e as ApiError).message).toBe("invalid input");
    }
  });

  it("extracts message field from JSON body", async () => {
    const response = mockResponse(404, "Not Found", JSON.stringify({ message: "not found" }));
    try {
      await throwApiError(response as unknown as Response);
      fail("should throw");
    } catch (e) {
      expect((e as ApiError).status).toBe(404);
      expect((e as ApiError).message).toBe("not found");
    }
  });

  it("uses a short fallback when JSON has no human-readable message", async () => {
    const body = { code: "ERR_UNKNOWN", details: "something" };
    const response = mockResponse(500, "Internal Server Error", JSON.stringify(body));
    try {
      await throwApiError(response as unknown as Response);
      fail("should throw");
    } catch (e) {
      expect((e as ApiError).status).toBe(500);
      expect((e as ApiError).message).toBe(
        "The service is temporarily unavailable. Please try again.",
      );
    }
  });

  it("never exposes gateway HTML and preserves correlation headers", async () => {
    const response = {
      ...mockResponse(
        502,
        "Bad Gateway",
        "<!DOCTYPE html><html><body>private IP and huge gateway page</body></html>",
      ),
      headers: new Headers({ "x-request-id": "request-123", "cf-ray": "ray-456" }),
    };
    await expect(throwApiError(response as unknown as Response)).rejects.toMatchObject({
      status: 502,
      message: "The service is temporarily unavailable. Please try again.",
      requestId: "request-123",
      rayId: "ray-456",
    });
  });

  it("extracts problem details while ignoring non-string and HTML messages", async () => {
    for (const detail of [{ unexpected: "object" }, "<html>Gateway error</html>"]) {
      const response = mockResponse(502, "Bad Gateway", JSON.stringify({ detail }));
      await expect(throwApiError(response as unknown as Response)).rejects.toMatchObject({
        message: "The service is temporarily unavailable. Please try again.",
      });
    }
    const response = mockResponse(
      409,
      "Conflict",
      JSON.stringify({ detail: "Reconnect your workspace", code: "slack_reconnect_required" }),
    );
    await expect(throwApiError(response as unknown as Response)).rejects.toMatchObject({
      message: "Reconnect your workspace",
      code: "slack_reconnect_required",
    });
  });

  it("bounds oversized plain-text responses", async () => {
    const response = mockResponse(502, "Bad Gateway", "x".repeat(10000));
    await expect(throwApiError(response as unknown as Response)).rejects.toMatchObject({
      message: "The service is temporarily unavailable. Please try again.",
    });
  });

  it("uses plain text body when not JSON", async () => {
    const response = mockResponse(502, "Bad Gateway", "upstream timeout");
    try {
      await throwApiError(response as unknown as Response);
      fail("should throw");
    } catch (e) {
      expect((e as ApiError).status).toBe(502);
      expect((e as ApiError).message).toBe("upstream timeout");
    }
  });

  it("handles empty body", async () => {
    const response = mockResponse(503, "Service Unavailable", "");
    try {
      await throwApiError(response as unknown as Response);
      fail("should throw");
    } catch (e) {
      expect((e as ApiError).status).toBe(503);
      expect((e as ApiError).statusText).toBe("Service Unavailable");
    }
  });

  it("handles text() throwing", async () => {
    const response = {
      status: 500,
      statusText: "Internal Server Error",
      text: () => Promise.reject(new Error("body consumed")),
    };
    try {
      await throwApiError(response as unknown as Response);
      fail("should throw");
    } catch (e) {
      expect(e).toBeInstanceOf(ApiError);
      expect((e as ApiError).status).toBe(500);
    }
  });
});
