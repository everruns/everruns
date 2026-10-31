/** Bound setup without imposing an ordinary request deadline on a live SSE body. */
export async function fetchUpstream(
  url: string,
  init: RequestInit,
  stream: boolean,
  deadlineMs = 30000,
): Promise<Response> {
  const deadline = new AbortController();
  const timer = setTimeout(
    () => deadline.abort(new DOMException("Upstream timed out", "TimeoutError")),
    deadlineMs,
  );
  timer.unref();
  try {
    const response = await fetch(url, {
      ...init,
      signal: AbortSignal.any([...(init.signal ? [init.signal] : []), deadline.signal]),
    });
    if (
      stream &&
      response.ok &&
      response.headers.get("content-type")?.split(";", 1)[0].trim().toLowerCase() ===
        "text/event-stream"
    ) {
      // Authentication and headers have completed. Browser cancellation and the
      // server's bounded SSE lifecycle still release this upstream connection.
      clearTimeout(timer);
    }
    return response;
  } catch (error) {
    clearTimeout(timer);
    throw error;
  }
}
