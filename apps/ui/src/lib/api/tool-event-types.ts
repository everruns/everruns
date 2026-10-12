// Augments legacy-api-types.ts rather than growing it: that file is on the source-size ratchet.
import type { OpenApiToolCompletedData } from "./schema-types";

declare module "./legacy-api-types" {
  interface ToolCompletedData {
    /**
     * Arguments the tool actually ran with; present only when `pre_tool_use` hooks rewrote the
     * model-authored arguments. A JSON value, or a truncated JSON string when
     * `executed_arguments_truncated` is set. Credential-named values arrive as `[REDACTED]`.
     */
    executed_arguments?: OpenApiToolCompletedData["executed_arguments"];
    /** True when `executed_arguments` is a truncated preview. */
    executed_arguments_truncated?: OpenApiToolCompletedData["executed_arguments_truncated"];
    /**
     * Details for people (chat details view, Session Trace) that the model never saw. Web
     * search puts the provider that answered, every attempt and the raw responses here.
     */
    details?: OpenApiToolCompletedData["details"];
  }
}
