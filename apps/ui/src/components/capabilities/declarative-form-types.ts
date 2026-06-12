// Local form state for the declarative capability editor and helpers to convert
// between the form shape and the API `DeclarativeCapabilityDefinition`.

import type {
  DeclarativeCapabilityDefinition,
  DeclarativeCapabilityFile,
  DeclarativeCapabilitySkill,
} from "@/lib/api/types";
import type { KeyValuePair } from "./key-value-editor";

let counter = 0;
// Stable local key for list rendering; never persisted.
function localId(): string {
  counter += 1;
  return `local-${counter}-${Date.now()}`;
}

export interface McpServerEntry {
  id: string;
  name: string;
  transportType: "http" | "stdio";
  url: string;
  headers: KeyValuePair[];
  command: string;
  args: string; // one argument per line
  env: KeyValuePair[];
  authMode: "none" | "api_key" | "oauth";
  oauthProviderId: string;
  toolDiscovery: boolean;
}

export interface SkillEntry {
  id: string;
  name: string;
  description: string;
  instructions: string;
  files: Array<{ id: string; path: string; content: string }>;
  userInvocable: boolean;
  disableModelInvocation: boolean;
}

export interface FileEntry {
  id: string;
  path: string;
  content: string;
  access: "readonly" | "readwrite";
}

export function newMcpServerEntry(): McpServerEntry {
  return {
    id: localId(),
    name: "",
    transportType: "http",
    url: "",
    headers: [],
    command: "",
    args: "",
    env: [],
    authMode: "none",
    oauthProviderId: "",
    toolDiscovery: true,
  };
}

export function newSkillEntry(): SkillEntry {
  return {
    id: localId(),
    name: "",
    description: "",
    instructions: "",
    files: [],
    userInvocable: true,
    disableModelInvocation: false,
  };
}

export function newFileEntry(): FileEntry {
  return { id: localId(), path: "", content: "", access: "readonly" };
}

function pairsToRecord(pairs: KeyValuePair[]): Record<string, string> {
  return Object.fromEntries(pairs.filter((p) => p.key).map((p) => [p.key, p.value]));
}

function recordToPairs(record: Record<string, string> | undefined): KeyValuePair[] {
  if (!record) return [];
  return Object.entries(record).map(([key, value]) => ({ key, value: String(value) }));
}

// --- MCP conversions -------------------------------------------------------

export function mcpServersToEntries(
  servers: Record<string, unknown> | undefined,
): McpServerEntry[] {
  if (!servers) return [];
  return Object.entries(servers).map(([name, raw]) => {
    const s = (raw ?? {}) as Record<string, unknown>;
    const transport = (s.type ?? s.transport_type ?? "http") as string;
    return {
      id: localId(),
      name,
      transportType: transport === "stdio" ? "stdio" : "http",
      url: typeof s.url === "string" ? s.url : "",
      headers: recordToPairs(s.headers as Record<string, string> | undefined),
      command: typeof s.command === "string" ? s.command : "",
      args: Array.isArray(s.args) ? (s.args as string[]).join("\n") : "",
      env: recordToPairs(s.env as Record<string, string> | undefined),
      authMode: (s.auth_mode as McpServerEntry["authMode"]) ?? "none",
      oauthProviderId: typeof s.oauth_provider_id === "string" ? s.oauth_provider_id : "",
      toolDiscovery: s.tool_discovery !== false,
    };
  });
}

function mcpEntryToValue(entry: McpServerEntry): Record<string, unknown> {
  const value: Record<string, unknown> = { type: entry.transportType };
  if (entry.transportType === "http") {
    if (entry.url.trim()) value.url = entry.url.trim();
    const headers = pairsToRecord(entry.headers);
    if (Object.keys(headers).length > 0) value.headers = headers;
  } else {
    if (entry.command.trim()) value.command = entry.command.trim();
    const args = entry.args
      .split("\n")
      .map((a) => a.trim())
      .filter(Boolean);
    if (args.length > 0) value.args = args;
    const env = pairsToRecord(entry.env);
    if (Object.keys(env).length > 0) value.env = env;
  }
  if (entry.authMode !== "none") value.auth_mode = entry.authMode;
  if (entry.authMode === "oauth" && entry.oauthProviderId.trim()) {
    value.oauth_provider_id = entry.oauthProviderId.trim();
  }
  if (!entry.toolDiscovery) value.tool_discovery = false;
  return value;
}

export function entriesToMcpServers(
  entries: McpServerEntry[],
): Record<string, unknown> | undefined {
  const named = entries.filter((e) => e.name.trim());
  if (named.length === 0) return undefined;
  return Object.fromEntries(named.map((e) => [e.name.trim(), mcpEntryToValue(e)]));
}

// --- Skill conversions -----------------------------------------------------

export function skillsToEntries(skills: DeclarativeCapabilitySkill[] | undefined): SkillEntry[] {
  if (!skills) return [];
  return skills.map((skill) => ({
    id: localId(),
    name: skill.name,
    description: skill.description,
    instructions: skill.instructions,
    files: (skill.files ?? []).map((f) => ({ id: localId(), path: f.path, content: f.content })),
    userInvocable: skill.user_invocable !== false,
    disableModelInvocation: skill.disable_model_invocation === true,
  }));
}

export function entriesToSkills(entries: SkillEntry[]): DeclarativeCapabilitySkill[] {
  return entries
    .filter((e) => e.name.trim())
    .map((e) => ({
      name: e.name.trim(),
      description: e.description,
      instructions: e.instructions,
      files: e.files
        .filter((f) => f.path.trim())
        .map((f) => ({ path: f.path.trim(), content: f.content })),
      user_invocable: e.userInvocable,
      disable_model_invocation: e.disableModelInvocation,
    }));
}

// --- File conversions ------------------------------------------------------

export function filesToEntries(files: DeclarativeCapabilityFile[] | undefined): FileEntry[] {
  if (!files) return [];
  return files.map((f) => ({
    id: localId(),
    path: f.path,
    content: f.content,
    access: f.access ?? "readonly",
  }));
}

export function entriesToFiles(entries: FileEntry[]): DeclarativeCapabilityFile[] {
  return entries
    .filter((e) => e.path.trim())
    .map((e) => ({ path: e.path.trim(), content: e.content, access: e.access }));
}

// --- Top-level form state --------------------------------------------------

export interface DeclarativeFormState {
  name: string;
  displayName: string;
  description: string;
  systemPrompt: string;
  riskLevel: "low" | "medium" | "high";
  mcpServers: McpServerEntry[];
  skills: SkillEntry[];
  files: FileEntry[];
}

export function definitionToFormState(
  def: DeclarativeCapabilityDefinition | undefined,
): DeclarativeFormState {
  return {
    name: def?.name ?? "",
    displayName: def?.display_name ?? "",
    description: def?.description ?? "",
    systemPrompt: def?.system_prompt ?? "",
    riskLevel: def?.risk_level ?? "low",
    mcpServers: mcpServersToEntries(def?.mcp_servers),
    skills: skillsToEntries(def?.skills),
    files: filesToEntries(def?.files),
  };
}

export function formStateToDefinition(
  state: DeclarativeFormState,
): DeclarativeCapabilityDefinition {
  return {
    name: state.name.trim(),
    display_name: state.displayName.trim() || undefined,
    description: state.description.trim(),
    category: "Declarative",
    icon: "puzzle",
    system_prompt: state.systemPrompt.trim() || undefined,
    mcp_servers: entriesToMcpServers(state.mcpServers),
    skills: entriesToSkills(state.skills),
    files: entriesToFiles(state.files),
    dependencies: ["session_file_system", "skills"],
    risk_level: state.riskLevel,
  };
}
