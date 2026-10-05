import type { SandboxPolicy } from "./schema-types";

declare module "./legacy-api-types" {
  interface Agent {
    /** Policy for selecting the primary Sandbox Template for new Sessions. */
    sandbox_policy?: SandboxPolicy | null;
  }

  interface CreateAgentRequest {
    /** Policy for selecting the primary Sandbox Template for new Sessions. */
    sandbox_policy?: SandboxPolicy;
  }

  interface UpdateAgentRequest {
    /** Sandbox policy; null clears the current policy. */
    sandbox_policy?: SandboxPolicy | null;
  }
}
