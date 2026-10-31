// Rendering fixture only. Consumer authorization is covered by the real
// server integration suite, not simulated by this server.
import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const avatar = readFileSync(
  join(
    dirname(fileURLToPath(import.meta.url)),
    "../../../crates/server/assets/avatar-presets/familiars-patch.png",
  ),
);
const logo = "/v1/avatars/avatar_01933b5a000070008000000000000001/circle-64.png";
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
        logo,
      }),
    );
    return;
  }
  if (request.url?.endsWith(logo)) {
    response.setHeader("Content-Type", "image/png");
    response.end(avatar);
    return;
  }
  response.statusCode = 404;
  response.end();
}).listen(9141, "127.0.0.1");
