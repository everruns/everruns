// Buffer framing before CRLF normalization: either byte of a delimiter can
// arrive in the next chunk. Completion events remain authoritative after replay.
export async function consumeSse(
  body: ReadableStream<Uint8Array>,
  receive: (event: string, data: string) => void,
) {
  const reader = body.getReader(),
    decoder = new TextDecoder();
  let buffer = "";
  try {
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) break;
      buffer += decoder.decode(chunk.value, { stream: true });
      let match: RegExpExecArray | null;
      while ((match = /\r?\n\r?\n/.exec(buffer))) {
        const frame = buffer.slice(0, match.index);
        buffer = buffer.slice(match.index + match[0].length);
        if (frame.length > 8 * 1024 * 1024) throw new Error("Conversation update is too large");
        const lines = frame.split(/\r?\n/);
        const data = lines
          .filter((line) => line.startsWith("data:"))
          .map((line) => line.slice(5).replace(/^ /, ""))
          .join("\n");
        const event =
          lines
            .find((line) => line.startsWith("event:"))
            ?.slice(6)
            .trim() ?? "message";
        if (data) receive(event, data);
      }
      if (buffer.length > 8 * 1024 * 1024) throw new Error("Conversation update is too large");
    }
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}
