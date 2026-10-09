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
     * Severity of a failed call, absent on success. A failed call goes back to the model, so it is
     * an `issue`; failures recorded before the field existed omit it and read as issues too.
     */
    severity?: OpenApiToolCompletedData["severity"];
  }
}
