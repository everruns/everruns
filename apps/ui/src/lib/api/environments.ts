// Environment types, from the generated OpenAPI schema.
//
// The aliases keep call sites readable without giving the UI a second opinion
// about the wire shape.

import type { EnvironmentCapabilities, SessionEnvironmentResponse } from "./types";

export type SessionEnvironment = SessionEnvironmentResponse;
export type { EnvironmentCapabilities };
export type { EnvironmentTargetDescriptor } from "./types";
