import "@testing-library/jest-dom";

// jsdom does not expose TextEncoder/TextDecoder from Node's util module.
// Polyfill them so code using the Encoding API works in tests.
import { TextDecoder, TextEncoder } from "util";
Object.assign(global, { TextDecoder, TextEncoder });
