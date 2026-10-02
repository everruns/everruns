// Guided agent templates: the pure parts of the setup path, kept out of the
// page so they are testable. The template definitions themselves live on the
// server (`crates/server/src/agent_templates.rs`).

import type {
  AgentCapabilityConfig,
  CapabilityId,
  CreateAgentTriggerRequest,
} from "@/lib/api/types";
import type {
  AgentExampleSetting,
  AgentExampleSetup,
  GuidedAgentExample,
} from "@/lib/api/agent-examples";

export function templateSetupPath(agentId: string, exampleName: string): string {
  return `/agents/${agentId}/setup?template=${encodeURIComponent(exampleName)}`;
}

/// Where to go after importing `exampleName`: its guided setup when it has one.
export function importedExampleLanding(
  examples: GuidedAgentExample[] | undefined,
  exampleName: string,
  agentId: string,
): string {
  const example = examples?.find((candidate) => candidate.name === exampleName);
  return example?.setup ? templateSetupPath(agentId, exampleName) : `/agents/${agentId}`;
}

function replaceDeep(value: unknown, placeholder: string, replacement: string): unknown {
  if (typeof value === "string") return value.split(placeholder).join(replacement);
  if (Array.isArray(value)) return value.map((item) => replaceDeep(item, placeholder, replacement));
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [
        key,
        replaceDeep(item, placeholder, replacement),
      ]),
    );
  }
  return value;
}

/// The template's trigger request for `repository` (`owner/name`).
export function buildTemplateTrigger(
  setup: AgentExampleSetup,
  repository: string,
): CreateAgentTriggerRequest {
  return replaceDeep(
    setup.trigger,
    setup.repository_placeholder,
    repository,
  ) as CreateAgentTriggerRequest;
}

export function defaultSettingValues(settings: AgentExampleSetting[]): Record<string, boolean> {
  return Object.fromEntries(settings.map((setting) => [setting.key, setting.default]));
}

/// Capabilities with each setting written to its capability config. Returns
/// `null` when nothing changes, so setup does not rewrite the agent needlessly.
export function applyTemplateSettings(
  capabilities: AgentCapabilityConfig[],
  settings: AgentExampleSetting[],
  values: Record<string, boolean>,
): AgentCapabilityConfig[] | null {
  let changed = false;
  const next = capabilities.map((capability) => {
    const relevant = settings.filter((setting) => setting.capability === capability.ref);
    if (relevant.length === 0) return capability;
    const config = { ...(capability.config ?? {}) };
    for (const setting of relevant) {
      const value = values[setting.key] ?? setting.default;
      if (config[setting.config_key] !== value) {
        config[setting.config_key] = value;
        changed = true;
      }
    }
    return { ref: capability.ref as CapabilityId, config };
  });
  return changed ? next : null;
}

/// The setup can create its trigger once GitHub is connected (when needed) and
/// a repository is picked.
export function canFinishSetup(
  setup: AgentExampleSetup,
  githubConnected: boolean,
  repository: string,
): boolean {
  return (!setup.connect_github || githubConnected) && /^[^/\s]+\/[^/\s]+$/.test(repository);
}
