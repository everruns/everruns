// Augments legacy-api-types.ts rather than growing it: that file is on the source-size ratchet.
import type { WebSearchDefaults } from "./schema-types";

declare module "./legacy-api-types" {
  interface UpdateOrganizationRequest {
    /** Web search defaults; replaces the whole object. */
    web_search_defaults?: WebSearchDefaults;
  }
}
