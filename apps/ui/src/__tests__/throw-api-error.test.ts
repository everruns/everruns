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

  it("falls back to stringified JSON when no error/message field", async () => {
    const body = { code: "ERR_UNKNOWN", details: "something" };
    const response = mockResponse(500, "Internal Server Error", JSON.stringify(body));
    try {
      await throwApiError(response as unknown as Response);
      fail("should throw");
    } catch (e) {
      expect((e as ApiError).status).toBe(500);
      expect((e as ApiError).message).toBe(JSON.stringify(body));
    }
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
