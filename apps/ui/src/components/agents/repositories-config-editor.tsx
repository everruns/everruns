"use client";

// Editor for the `repositories` capability config: the repositories every new
// session of this agent checks out, what the agent may add on its own, and the
// branch its work goes to. Repository choices come from the agent's GitHub
// identity, the same identity that clones and pushes.

import Link from "next/link";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SegmentedControl } from "@/components/ui/segmented-control";
import { RepositorySetEditor } from "@/components/repositories/repository-set-editor";
import { useAgentGitHub, useGitHubRepositories } from "@/hooks/use-agent-github";
import {
  REPOSITORIES_DEFAULTS,
  readRepositoriesConfig,
  writeRepositoriesConfig,
  type AddRepositories,
  type RepositoriesConfig,
} from "@/lib/repositories-config";
import { useUiTranslations } from "@/providers/locale-provider";
import { useCapabilityEditorContext } from "./capability-editor-context";

/** The identity line: who clones and pushes, and where to change that. */
function GitHubIdentity({ agentId }: { agentId: string }) {
  const ui = useUiTranslations();
  const { data: status, isLoading } = useAgentGitHub(agentId);
  if (isLoading) return null;
  if (!status?.connected) {
    return (
      <p className="text-xs text-muted-foreground">
        {ui(
          "This agent has no GitHub identity yet, so it cannot clone private repositories or push.",
        )}{" "}
        <Link className="underline" href={`/agents/${agentId}/github`}>
          {ui("Connect GitHub")}
        </Link>
      </p>
    );
  }
  const name =
    status.kind === "everruns_app"
      ? ui("Everruns GitHub App")
      : (status.bot_login ?? status.app_slug);
  return (
    <p className="text-xs text-muted-foreground">
      {ui("GitHub identity:")} <span className="text-foreground">{name}</span>
      {status.account ? ui(" · installed on {account}", { account: status.account }) : ""}
      {status.missing_permissions.length > 0 && (
        <span className="block text-destructive">
          {ui("Missing permissions: {list}", {
            list: status.missing_permissions.join(", "),
          })}
        </span>
      )}
    </p>
  );
}

export function RepositoriesConfigEditor({
  config,
  onChange,
  disabled,
}: {
  config: unknown;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}) {
  const ui = useUiTranslations();
  const { agentId } = useCapabilityEditorContext();
  const current = readRepositoriesConfig(config);
  const { data: status } = useAgentGitHub(agentId ?? "", !!agentId);
  const { data: available } = useGitHubRepositories(status?.identity_id, !!status?.connected);

  const update = (patch: Partial<RepositoriesConfig>) =>
    onChange(writeRepositoriesConfig({ ...current, ...patch }));

  return (
    <div className="space-y-4 pt-2">
      <div className="space-y-1.5">
        <Label className="text-xs font-normal text-muted-foreground">
          {ui("Default repositories")}
        </Label>
        <p className="text-xs text-muted-foreground">
          {ui("Checked out side by side in every new session.")}
        </p>
        <RepositorySetEditor
          idPrefix="repositories-config"
          entries={current.repositories}
          onChange={(repositories) => update({ repositories })}
          available={available}
          showBranch
          disabled={disabled}
        />
      </div>

      <div className="space-y-1.5">
        <Label className="text-xs font-normal text-muted-foreground">
          {ui("Agent may add repositories")}
        </Label>
        <p className="text-xs text-muted-foreground">
          {ui("During a session, when the work needs another one.")}
        </p>
        <SegmentedControl
          label={ui("Agent may add repositories")}
          value={current.add_repositories}
          onValueChange={(next) => update({ add_repositories: next as AddRepositories })}
          options={[
            { value: "off", label: ui("Off") },
            { value: "read", label: ui("Read only") },
            { value: "write", label: ui("Read and write") },
          ]}
          disabled={disabled}
        />
      </div>

      <div className="space-y-1.5">
        <Label
          htmlFor="repositories-branch-prefix"
          className="text-xs font-normal text-muted-foreground"
        >
          {ui("Work branch prefix")}
        </Label>
        <p className="text-xs text-muted-foreground">
          {ui("The agent pushes only to its own branch, never by force.")}
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <Input
            id="repositories-branch-prefix"
            value={current.branch_prefix}
            onChange={(event) => update({ branch_prefix: event.target.value })}
            placeholder={REPOSITORIES_DEFAULTS.branch_prefix}
            disabled={disabled}
            className="h-8 w-40 font-mono text-[13px]"
          />
          <span className="font-mono text-xs text-muted-foreground">
            → {current.branch_prefix}bdaf0ce2
          </span>
        </div>
      </div>

      <details>
        <summary className="cursor-pointer text-xs text-muted-foreground">
          {ui("Commit author and history")}
        </summary>
        <div className="space-y-3 pt-3">
          <div className="space-y-1.5">
            <Label
              htmlFor="repositories-commit-name"
              className="text-xs font-normal text-muted-foreground"
            >
              {ui("Commit author name")}
            </Label>
            <Input
              id="repositories-commit-name"
              value={current.commit_name}
              onChange={(event) => update({ commit_name: event.target.value })}
              disabled={disabled}
              className="h-8"
            />
          </div>
          <div className="space-y-1.5">
            <Label
              htmlFor="repositories-commit-email"
              className="text-xs font-normal text-muted-foreground"
            >
              {ui("Commit author email")}
            </Label>
            <Input
              id="repositories-commit-email"
              value={current.commit_email}
              onChange={(event) => update({ commit_email: event.target.value })}
              disabled={disabled}
              className="h-8"
            />
          </div>
          <div className="space-y-1.5">
            <Label
              htmlFor="repositories-depth"
              className="text-xs font-normal text-muted-foreground"
            >
              {ui("History depth, in commits")}
            </Label>
            <Input
              id="repositories-depth"
              type="number"
              min={1}
              max={10000}
              value={current.depth}
              onChange={(event) => {
                const depth = Number(event.target.value);
                if (Number.isInteger(depth) && depth >= 1 && depth <= 10000) update({ depth });
              }}
              disabled={disabled}
              className="h-8 w-32"
            />
          </div>
        </div>
      </details>

      {agentId && <GitHubIdentity agentId={agentId} />}
    </div>
  );
}
