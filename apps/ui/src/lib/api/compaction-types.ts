import type { components } from "./generated/openapi";

/** Safe compaction inspection metadata; checkpoint contents stay private. */
export type CompactionStatus = components["schemas"]["CompactionStatus"];

export type ContextCompactedData = components["schemas"]["ContextCompactedData"];

/** Attribution for native work; billing is recorded on its separate generation. */
export type LlmCompactionInfo = components["schemas"]["LlmCompactionInfo"];
