// A session stream stays open for minutes and sends heartbeats. A short
// upstream abort resets the browser connection. On HTTP/3 that reset is
// reported as a network error, and the next attempt can fail the same way.
export function liveStream(path: string): boolean {
  return path.endsWith("/sse");
}

export function proxyHeaders(
  path: string,
  contentType: string | null,
): Record<string, string> {
  if (liveStream(path)) {
    return {
      "Content-Type": contentType ?? "text/event-stream",
      // no-transform asks intermediaries not to buffer or recompress the stream.
      "Cache-Control": "no-cache, no-transform",
      "X-Accel-Buffering": "no",
    };
  }
  return {
    "Content-Type": contentType ?? "application/json",
    "Cache-Control": "no-store",
    "X-Accel-Buffering": "no",
  };
}
