// Native executors need only the small command-reducer Wasm. Isolate Engines
// install the same exports from their own module and import service.js directly.
import "./rust.js";
export { default, SessionAuthority, authenticate, address, readJson, failure } from "./service.js";
