"use client";

// Changes — what a coding session did in each repository it works on.
//
// Read from the sandbox, the same source as the agent's repository status
// tool. Nothing here merges, pushes or discards: the agent acts, the person
// reviews. A sandbox that is not running is reported, never started.

import { useMemo } from "react";
import { ExternalLink, RefreshCw } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button, buttonVariants } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import type { SessionRepository, SessionRepositoryFile } from "@/lib/api/sandboxes";
import { cn } from "@/lib/utils";
import { useUiTranslations } from "@/providers/locale-provider";

type Translate = ReturnType<typeof useUiTranslations>;

export interface RepositoryPullRequest {
  number: number;
  url: string;
}

/** A pull request URL the session's tools returned for `repo`, newest first. */
export function findPullRequest(
  repo: string | null | undefined,
  toolResultTexts: string[],
): RepositoryPullRequest | undefined {
  if (!repo) return undefined;
  const pattern = new RegExp(
    `https://github\\.com/${repo.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}/pull/(\\d+)`,
    "i",
  );
  for (let index = toolResultTexts.length - 1; index >= 0; index -= 1) {
    const match = pattern.exec(toolResultTexts[index]);
    if (match) return { number: Number(match[1]), url: match[0] };
  }
  return undefined;
}

type RepositoryState = "pushed" | "pending" | "unchanged";

export function repositoryState(repository: SessionRepository): RepositoryState {
  const ahead = repository.commits_ahead_of_base ?? 0;
  const changed =
    ahead > 0 || repository.uncommitted_files > 0 || (repository.files?.length ?? 0) > 0;
  if (!changed) return "unchanged";
  const pushed = repository.commits_not_pushed === 0 && repository.uncommitted_files === 0;
  return pushed ? "pushed" : "pending";
}

/** One line a reviewer reads first: how far the work is and whether it left the sandbox. */
export function repositorySummary(repository: SessionRepository, ui: Translate): string {
  const parts: string[] = [];
  const ahead = repository.commits_ahead_of_base ?? 0;
  const state = repositoryState(repository);
  if (state === "unchanged") {
    parts.push(ui("no changes"));
  } else {
    if (ahead > 0) {
      parts.push(
        ahead === 1
          ? ui("1 commit ahead of {base}", { base: repository.base_branch })
          : ui("{count} commits ahead of {base}", {
              count: ahead,
              base: repository.base_branch,
            }),
      );
    }
    if (repository.uncommitted_files > 0) {
      parts.push(
        repository.uncommitted_files === 1
          ? ui("1 file not committed")
          : ui("{count} files not committed", {
              count: repository.uncommitted_files,
            }),
      );
    }
    if (repository.access === "write") {
      parts.push(state === "pushed" ? ui("pushed") : ui("not pushed"));
    }
  }
  if (repository.access !== "write") {
    parts.push(ui("read only, the agent cannot push here"));
  }
  return parts.join(" · ");
}

const STATE_DOT: Record<RepositoryState, string> = {
  pushed: "bg-success",
  pending: "bg-warning",
  unchanged: "bg-muted-foreground/40",
};

function FileRow({ file }: { file: SessionRepositoryFile }) {
  const ui = useUiTranslations();
  return (
    <li className="flex items-baseline justify-between gap-4 py-1">
      <span className="min-w-0 truncate font-mono text-sm">{file.path}</span>
      <span className="shrink-0 font-mono text-xs">
        {file.additions != null && <span className="text-success">+{file.additions}</span>}{" "}
        {file.deletions != null && <span className="text-destructive">−{file.deletions}</span>}
        {!file.committed && (
          <span className="ml-2 text-muted-foreground">{ui("not committed")}</span>
        )}
      </span>
    </li>
  );
}

function Diff({ diff, truncated }: { diff: string; truncated: boolean }) {
  const ui = useUiTranslations();
  return (
    <details className="mt-2">
      <summary className="cursor-pointer text-sm text-muted-foreground">{ui("Show diff")}</summary>
      <pre className="mt-2 max-h-[32rem] overflow-auto rounded-md border border-border bg-muted/40 py-2 font-mono text-xs leading-5">
        {diff.split("\n").map((line, index) => (
          <div
            key={index}
            className={cn(
              "px-3 whitespace-pre",
              line.startsWith("+") && !line.startsWith("+++") && "bg-success/10",
              line.startsWith("-") && !line.startsWith("---") && "bg-destructive/10",
              (line.startsWith("@@") || line.startsWith("diff --git")) && "text-muted-foreground",
            )}
          >
            {line || " "}
          </div>
        ))}
      </pre>
      {truncated && (
        <p className="mt-1 text-xs text-muted-foreground">
          {ui("The diff is cut here. The file list above is complete.")}
        </p>
      )}
    </details>
  );
}

function RepositorySection({
  repository,
  pullRequest,
}: {
  repository: SessionRepository;
  pullRequest?: RepositoryPullRequest;
}) {
  const ui = useUiTranslations();
  const state = repositoryState(repository);
  const name = repository.repo ?? repository.path;
  return (
    <section className="border-b border-border px-6 py-4 last:border-b-0" aria-label={name}>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
        <span className={cn("size-2 shrink-0", STATE_DOT[state])} aria-hidden />
        <h3 className="font-mono text-sm font-semibold">{name}</h3>
        <Badge variant="outline" className="uppercase">
          {repository.access === "write" ? ui("Write") : ui("Read")}
        </Badge>
        <span className="font-mono text-sm text-muted-foreground">{repository.branch}</span>
        <span className="text-sm text-muted-foreground">· {repositorySummary(repository, ui)}</span>
        <span className="ml-auto">
          {pullRequest ? (
            <a
              href={pullRequest.url}
              target="_blank"
              rel="noreferrer"
              className={buttonVariants({ variant: "outline", size: "sm" })}
            >
              {ui("View pull request #{number}", {
                number: pullRequest.number,
              })}
              <ExternalLink className="size-3.5" />
            </a>
          ) : (
            state === "pending" &&
            repository.access === "write" && (
              <span className="text-sm text-muted-foreground">
                {ui("The agent pushes; you review")}
              </span>
            )
          )}
        </span>
      </div>
      {(repository.files?.length ?? 0) > 0 && (
        <div className="mt-3 pl-5">
          <ul>
            {(repository.files ?? []).map((file) => (
              <FileRow key={file.path} file={file} />
            ))}
          </ul>
          {repository.diff && (
            <Diff diff={repository.diff} truncated={repository.diff_truncated === true} />
          )}
        </div>
      )}
    </section>
  );
}

export interface SessionRepositoriesProps {
  sandboxState?: string;
  repositories?: SessionRepository[];
  /** Text of the session's tool results, oldest first, searched for pull request links. */
  toolResultTexts: string[];
  loading: boolean;
  refreshing: boolean;
  error?: string;
  onRefresh: () => void;
}

export function SessionRepositories({
  sandboxState,
  repositories,
  toolResultTexts,
  loading,
  refreshing,
  error,
  onRefresh,
}: SessionRepositoriesProps) {
  const ui = useUiTranslations();
  const pullRequests = useMemo(
    () =>
      new Map(
        (repositories ?? []).map((repository) => [
          repository.path,
          findPullRequest(repository.repo, toolResultTexts),
        ]),
      ),
    [repositories, toolResultTexts],
  );

  if (loading) {
    return (
      <div className="p-6">
        <Skeleton className="h-40 w-full" />
      </div>
    );
  }

  let notice: string | undefined;
  if (error) {
    notice = error;
  } else if (sandboxState === "none") {
    notice = ui("This session has no repositories.");
  } else if (sandboxState !== "running") {
    notice = ui(
      "The sandbox is not running, so there is nothing to read. Changes show here while it runs; pushed work stays on GitHub.",
    );
  } else if (!repositories?.length) {
    notice = ui("No repository is checked out yet. The agent checks them out when it starts work.");
  }

  return (
    <div className="flex-1 overflow-auto">
      <div className="flex items-center justify-between px-6 pt-4">
        <p className="text-sm text-muted-foreground">
          {ui("Read from the sandbox. The agent commits and pushes; you review.")}
        </p>
        <Button variant="outline" size="sm" onClick={onRefresh} disabled={refreshing}>
          <RefreshCw className={cn("size-3.5", refreshing && "animate-spin")} />
          {ui("Refresh")}
        </Button>
      </div>
      {notice ? (
        <p className="px-6 py-8 text-sm text-muted-foreground" role="status">
          {notice}
        </p>
      ) : (
        <div className="mt-2">
          {repositories!.map((repository) => (
            <RepositorySection
              key={repository.path}
              repository={repository}
              pullRequest={pullRequests.get(repository.path)}
            />
          ))}
        </div>
      )}
    </div>
  );
}
