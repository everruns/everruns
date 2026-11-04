import Rust from "./worker/shim.mjs";
import { DurableObject } from "cloudflare:workers";

// Both entry points execute the same Rust WASM and application HTTP adapter.
export class DriverObject extends DurableObject {
  fetch(request) {
    return Rust.prototype.fetch.call(this, request);
  }
}
export default class extends Rust {
  fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === "/health") return new Response("ok");
    if (path === "/do") {
      return this.env.TEST.get(this.env.TEST.idFromName("provider-test")).fetch(
        request,
      );
    }
    return super.fetch(request);
  }
}
