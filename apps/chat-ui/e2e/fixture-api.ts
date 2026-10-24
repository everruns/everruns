// Rendering fixture only. Consumer authorization is covered by the real
// server integration suite, not simulated by this server.
import { createServer } from "node:http";
let mode = "multi_chat";
createServer((request, response) => {
  if (request.url?.startsWith("/mode/")) {
    mode = request.url.split("/")[2];
    response.end("ok");
    return;
  }
  if (request.url?.endsWith("/chat/config")) {
    response.setHeader("Content-Type", "application/json");
    response.end(
      JSON.stringify({
        name: "Studio",
        description: "A thoughtful partner for research, writing, and getting work done.",
        interface_mode: mode,
        features: { threads: true, files: true },
      }),
    );
    return;
  }
  response.statusCode = 404;
  response.end();
}).listen(9141, "127.0.0.1");
