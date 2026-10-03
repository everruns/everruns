import type { EnvironmentSet } from "./schema-types";

declare module "./legacy-api-types" {
  interface Agent {
    /** Named execution environments offered when a new session starts. */
    environments?: EnvironmentSet | null;
  }

  interface CreateAgentRequest {
    /** Named execution environments offered when a new session starts. */
    environments?: EnvironmentSet;
  }

  interface UpdateAgentRequest {
    /** Named execution environments; null clears the current set. */
    environments?: EnvironmentSet | null;
  }
}
