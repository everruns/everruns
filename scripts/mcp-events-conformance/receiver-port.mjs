// Keep the official receiver on loopback and observe its ephemeral port.
// Only the test egress adapter uses this address; grading code is unchanged.
import http from "node:http";
import { writeFileSync, existsSync } from "node:fs";
const original = http.Server.prototype.listen;
http.Server.prototype.listen = function (...args) {
  if (args[1] === "0.0.0.0") args[1] = "127.0.0.1";
  this.once("listening", () => {
    const file = process.env.EVERRUNS_EVENTS_RECEIVER_FILE;
    const address = this.address();
    if (file && !existsSync(file) && address && typeof address === "object")
      writeFileSync(file, String(address.port));
  });
  return original.apply(this, args);
};
