"use client";

import { useAgentGitHub, useGitHubRepositories } from "@/hooks/use-agent-github";
import { ConnectGitHubButton } from "@/components/agents/connect-github-button";
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
import type { TriggerEventFilter } from "@/lib/api/schema-types";

/// One session per pull request, or a fresh one per event.
export type GitHubSessionMode = Extract<SessionBinding, "per_thread" | "session_per_invocation">;

export type GitHubTriggerFormState = {
  events: string[];
  repositories: string[];
  session_mode: GitHubSessionMode;
  message: string;
  enabled: boolean;
  /** Kept as saved so conditions set through the API survive an edit here. */
  filter: TriggerEventFilter | null;
};

/** Where CI events carry their normalized result. */
export const CI_OUTCOME_PATH = "github.ci.outcome";

const CI_EVENTS = ["workflow_run.completed", "check_suite.completed"];

export function onlyWhenOutcomeChanges(form: GitHubTriggerFormState): boolean {
  return form.filter?.changes_only?.path === CI_OUTCOME_PATH;
}

function withOutcomeChanges(form: GitHubTriggerFormState, on: boolean): GitHubTriggerFormState {
  const base: TriggerEventFilter = { conditions: [], ...form.filter };
  const { changes_only: _dropped, ...rest } = base;
  return {
    ...form,
    filter: on ? { ...rest, changes_only: { path: CI_OUTCOME_PATH } } : rest,
  };
}

export const GITHUB_EVENT_OPTIONS = [
  { value: "pull_request.opened", label: "Pull request opened" },
  { value: "pull_request.reopened", label: "Pull request reopened" },
  { value: "pull_request.synchronize", label: "Pull request updated (new commits)" },
  { value: "pull_request.ready_for_review", label: "Pull request ready for review" },
  { value: "pull_request.closed", label: "Pull request closed" },
  { value: "issue_comment.created", label: "Issue comment" },
  { value: "workflow_run.completed", label: "CI workflow finished" },
  { value: "check_suite.completed", label: "Check suite finished" },
];

export const DEFAULT_GITHUB_MESSAGE =
  "Summarize pull request {{github.repository}}#{{github.number}}: {{github.title}} ({{github.url}}). Read it with get_github_pull_request and get_github_pull_request_diff, then post or update your summary with upsert_github_comment.";

export const EMPTY_GITHUB_FORM: GitHubTriggerFormState = {
  events: GITHUB_EVENT_OPTIONS.slice(0, 4).map((option) => option.value),
  repositories: [],
  session_mode: "per_thread",
  message: DEFAULT_GITHUB_MESSAGE,
  enabled: true,
  filter: null,
};

/**
 * A new trigger prefilled from the page URL (`events`, `repos`, `changes_only`,
 * `message`), as the agent builder links it after creating the agent.
 */
export function githubFormFromParams(params: URLSearchParams | null): GitHubTriggerFormState {
  if (!params?.get("events")) return EMPTY_GITHUB_FORM;
  const known = GITHUB_EVENT_OPTIONS.map((option) => option.value);
  const list = (key: string) =>
    (params.get(key) ?? "")
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean);
  const events = list("events").filter((event) => known.includes(event));
  const form: GitHubTriggerFormState = {
    ...EMPTY_GITHUB_FORM,
    events: events.length > 0 ? events : EMPTY_GITHUB_FORM.events,
    repositories: list("repos"),
    session_mode: events.every((event) => CI_EVENTS.includes(event))
      ? "session_per_invocation"
      : "per_thread",
    message: params.get("message")?.trim() || EMPTY_GITHUB_FORM.message,
  };
  return withOutcomeChanges(form, params.get("changes_only") === "1");
}

/// What a GitHub trigger listens to, for its page subtitle.
export function githubEventsLabel(events: string[]): string {
  if (events.length > 0 && events.every((event) => CI_EVENTS.includes(event))) return "CI events";
  if (events.length > 0 && !events.some((event) => CI_EVENTS.includes(event))) {
    return "Pull request events";
  }
  return "GitHub events";
}

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
  returnTo,
}: {
  agentId: string;
  value: GitHubTriggerFormState;
  onChange: (next: GitHubTriggerFormState) => void;
  idPrefix?: string;
  /** Where GitHub sends the browser back after Connect; defaults to the agent page. */
  returnTo?: string;
}) {
  const { data: status, isLoading } = useAgentGitHub(agentId);
  const { data: repos = [] } = useGitHubRepositories(status?.identity_id, !!status?.connected);

  if (isLoading) return <p className="text-sm text-muted-foreground">Loading GitHub status…</p>;

  if (!status?.connected) {
    return (
      <div className="space-y-3 border p-4">
        <p className="text-sm text-muted-foreground">
          Connect GitHub to this agent before adding a GitHub trigger.
          {!status?.app_created && " The App can only be installed where it is created."}
        </p>
        <ConnectGitHubButton
          agentId={agentId}
          status={status}
          returnTo={returnTo}
          size="default"
          variant="default"
        />
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
      {value.events.some((event) => CI_EVENTS.includes(event)) && (
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <Checkbox
              id={`${idPrefix}-changes-only`}
              checked={onlyWhenOutcomeChanges(value)}
              onCheckedChange={(on) => onChange(withOutcomeChanges(value, on === true))}
            />
            <Label htmlFor={`${idPrefix}-changes-only`}>Only when the CI result changes</Label>
          </div>
          <p className="text-xs text-muted-foreground">
            Wakes on green to red and red to green for each workflow and branch, not on every run.
          </p>
        </div>
      )}
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
            <SelectItem value="per_thread">One session per pull request or CI workflow</SelectItem>
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
