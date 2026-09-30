"use client";

import {
  useAgentGitHub,
  useConnectAgentGitHub,
  useGitHubRepositories,
} from "@/hooks/use-agent-github";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import type { SessionBinding } from "@/lib/api/types";

/// One session per pull request, or a fresh one per event.
export type GitHubSessionMode = Extract<SessionBinding, "per_thread" | "session_per_invocation">;

export type GitHubTriggerFormState = {
  events: string[];
  repositories: string[];
  session_mode: GitHubSessionMode;
  message: string;
  enabled: boolean;
};

export const GITHUB_EVENT_OPTIONS = [
  { value: "pull_request.opened", label: "Pull request opened" },
  { value: "pull_request.reopened", label: "Pull request reopened" },
  { value: "pull_request.synchronize", label: "Pull request updated (new commits)" },
  { value: "pull_request.ready_for_review", label: "Pull request ready for review" },
  { value: "pull_request.closed", label: "Pull request closed" },
  { value: "issue_comment.created", label: "Issue comment" },
];

export const DEFAULT_GITHUB_MESSAGE =
  "Summarize pull request {{github.repository}}#{{github.number}}: {{github.title}} ({{github.url}}). Read it with get_github_pull_request and get_github_pull_request_diff, then post or update your summary with upsert_github_comment.";

export const EMPTY_GITHUB_FORM: GitHubTriggerFormState = {
  events: GITHUB_EVENT_OPTIONS.slice(0, 4).map((option) => option.value),
  repositories: [],
  session_mode: "per_thread",
  message: DEFAULT_GITHUB_MESSAGE,
  enabled: true,
};

export function isGitHubFormValid(form: GitHubTriggerFormState): boolean {
  return form.events.length > 0 && form.message.trim().length > 0;
}

function toggle(list: string[], item: string, on: boolean): string[] {
  return on ? [...list.filter((entry) => entry !== item), item] : list.filter((e) => e !== item);
}

/// Shows a Connect notice instead of the form until the agent has a GitHub App.
export function GitHubTriggerForm({
  agentId,
  value,
  onChange,
  idPrefix = "github-trigger",
}: {
  agentId: string;
  value: GitHubTriggerFormState;
  onChange: (next: GitHubTriggerFormState) => void;
  idPrefix?: string;
}) {
  const { data: status, isLoading } = useAgentGitHub(agentId);
  const connect = useConnectAgentGitHub(agentId);
  const { data: repos = [] } = useGitHubRepositories(status?.identity_id, !!status?.connected);

  if (isLoading) return <p className="text-sm text-muted-foreground">Loading GitHub status…</p>;

  if (!status?.connected) {
    return (
      <div className="space-y-3 border p-4">
        <p className="text-sm text-muted-foreground">
          Connect GitHub to this agent before adding a pull request trigger.
        </p>
        <Button type="button" onClick={() => connect.mutate()} disabled={connect.isPending}>
          {status?.app_created ? "Finish installing" : "Connect GitHub"}
        </Button>
      </div>
    );
  }

  return (
    <>
      <div className="space-y-2">
        <Label>Events</Label>
        <div className="space-y-2">
          {GITHUB_EVENT_OPTIONS.map((option) => (
            <div key={option.value} className="flex items-center gap-2">
              <Checkbox
                id={`${idPrefix}-event-${option.value}`}
                checked={value.events.includes(option.value)}
                onCheckedChange={(on) =>
                  onChange({ ...value, events: toggle(value.events, option.value, on) })
                }
              />
              <Label htmlFor={`${idPrefix}-event-${option.value}`}>{option.label}</Label>
            </div>
          ))}
        </div>
      </div>
      <div className="space-y-2">
        <Label>Repositories</Label>
        <p className="text-xs text-muted-foreground">None selected means all repositories.</p>
        <div className="max-h-48 space-y-2 overflow-y-auto">
          {repos.length === 0 && <p className="text-sm text-muted-foreground">All repositories</p>}
          {repos.map((repo) => (
            <div key={repo.full_name} className="flex items-center gap-2">
              <Checkbox
                id={`${idPrefix}-repo-${repo.full_name}`}
                checked={value.repositories.includes(repo.full_name)}
                onCheckedChange={(on) =>
                  onChange({
                    ...value,
                    repositories: toggle(value.repositories, repo.full_name, on),
                  })
                }
              />
              <Label htmlFor={`${idPrefix}-repo-${repo.full_name}`}>
                {repo.full_name}
                {repo.private ? " (private)" : ""}
              </Label>
            </div>
          ))}
        </div>
      </div>
      <div className="space-y-2">
        <Label htmlFor={`${idPrefix}-session-mode`}>Session mode</Label>
        <Select
          value={value.session_mode}
          onValueChange={(session_mode) =>
            onChange({ ...value, session_mode: session_mode as GitHubSessionMode })
          }
        >
          <SelectTrigger id={`${idPrefix}-session-mode`} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="per_thread">One session per pull request</SelectItem>
            <SelectItem value="session_per_invocation">New session per event</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div className="space-y-2">
        <Label htmlFor={`${idPrefix}-message`}>Message</Label>
        <Textarea
          id={`${idPrefix}-message`}
          value={value.message}
          onChange={(event) => onChange({ ...value, message: event.target.value })}
          rows={5}
        />
      </div>
      <div className="flex items-center gap-2">
        <Switch
          id={`${idPrefix}-enabled`}
          checked={value.enabled}
          onCheckedChange={(enabled) => onChange({ ...value, enabled })}
        />
        <Label htmlFor={`${idPrefix}-enabled`}>Enabled</Label>
      </div>
    </>
  );
}
