// Environment types, from the generated OpenAPI schema.
//
// The aliases keep call sites readable without giving the UI a second opinion
// about the wire shape.

import { api } from "./client";
import type {
  EnvironmentCapabilities,
  EnvironmentTargetsResponse,
  SessionEnvironmentResponse,
} from "./types";

export type SessionEnvironment = SessionEnvironmentResponse;
export type { EnvironmentCapabilities };
export type { EnvironmentTargetDescriptor } from "./types";

export async function listEnvironmentTargets(): Promise<EnvironmentTargetsResponse> {
  const response = await api.get<EnvironmentTargetsResponse>("/v1/environment-targets");
  return response.data;
}
