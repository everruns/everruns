"use client";

// Which repositories one session works on, chosen before it starts.
//
// Starts from the agent's usual set. Access can be turned down, and
// repositories added, only within what the agent's settings allow: a session
// never gets more than its agent. The picked set travels as the session's own
// `repositories` config, and only when it differs from the agent's.

import { RepositorySetEditor } from "@/components/repositories/repository-set-editor";
import { useAgentGitHub, useGitHubRepositories } from "@/hooks/use-agent-github";
import {
  allowedAccess,
  readRepositoriesConfig,
  type RepositoryEntry,
} from "@/lib/repositories-config";
import { useUiTranslations } from "@/providers/locale-provider";

export function SessionRepositoriesPicker({
  agentId,
  agentConfig,
  value,
  onChange,
  disabled,
}: {
  agentId: string;
  /** The agent's `repositories` capability config. */
  agentConfig: unknown;
  value: RepositoryEntry[];
  onChange: (value: RepositoryEntry[]) => void;
  disabled?: boolean;
}) {
  const ui = useUiTranslations();
  const agent = readRepositoriesConfig(agentConfig);
  const { data: status } = useAgentGitHub(agentId);
  const { data: available } = useGitHubRepositories(status?.identity_id, !!status?.connected);
  const canAdd = agent.add_repositories !== "off";

  return (
    <div className="space-y-1.5">
      <p className="text-sm font-medium">{ui("Repositories")}</p>
      {value.length === 0 && (
        <p className="text-xs text-muted-foreground">
          {canAdd
            ? ui("No repositories yet. Add the ones this chat should work on.")
            : ui("This agent has no repositories set.")}
        </p>
      )}
      <RepositorySetEditor
        idPrefix="session-repositories"
        entries={value}
        onChange={onChange}
        available={available}
        maxAccess={(repo) => allowedAccess(agent, repo)}
        canAdd={canAdd}
        disabled={disabled}
      />
      <p className="text-xs text-muted-foreground">
        {ui(
          "Starts from the agent's repositories. With write access the agent pushes its own branch and can open a pull request.",
        )}
      </p>
    </div>
  );
}
