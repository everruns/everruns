// Skill types (Agent Skills registry)

/** Skill source type */
export type SkillSourceType = "markdown" | "archive";

/** Skill status */
export type SkillStatus = "active" | "disabled" | "archived" | "deleted";

/** Skill entity */
export interface Skill {
  id: string;
  name: string;
  description: string;
  license?: string;
  compatibility?: string;
  metadata: Record<string, unknown>;
  allowed_tools?: string;
  source_type: SkillSourceType;
  status: SkillStatus;
  version: string;
  created_at: string;
  updated_at: string;
  archived_at: string | null;
  deleted_at: string | null;
}

/** Skill content (full instructions + files) */
export interface SkillContent {
  skill_md: string;
  files: SkillFileEntry[];
}

/** File entry in a skill archive */
export interface SkillFileEntry {
  path: string;
  content: string;
}

/** Per-skill usage counts (active agents/harnesses referencing the skill capability). */
export interface SkillUsage {
  agents: number;
  harnesses: number;
}

/** Map keyed by SkillId (e.g. `skill_<32hex>`); skills with zero usage are absent. */
export type SkillUsageMap = Record<string, SkillUsage>;

/** Validation result for SKILL.md */
export interface SkillValidationResult {
  valid: boolean;
  name?: string;
  description?: string;
  errors: string[];
  warnings: string[];
}

/** Request to create a skill from SKILL.md */
export interface CreateSkillRequest {
  skill_md: string;
}

/** Request to update a skill */
export interface UpdateSkillRequest {
  skill_md?: string;
  status?: SkillStatus;
}

/** Request to validate SKILL.md */
export interface ValidateSkillRequest {
  skill_md: string;
}
