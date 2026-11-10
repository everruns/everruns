"use client";

// Changes — per-repository view of what a coding session changed.

import { useMemo } from "react";
import { SessionRepositories } from "@/components/session/session-repositories";
import { useSessionRepositories } from "@/hooks/use-session-repositories";
import { useUiTranslations } from "@/providers/locale-provider";
import { useSessionContext } from "../session-context";

export default function SessionChangesPage() {
  const ui = useUiTranslations();
  const { sessionId, toolResultsMap } = useSessionContext();
  const { data, isLoading, isFetching, error, refetch } = useSessionRepositories(sessionId);
  // Pull request links come from what the agent's tools returned in this session.
  const toolResultTexts = useMemo(
    () =>
      [...toolResultsMap.values()]
        .filter((result) => result.tool_name === "create_github_pull_request")
        .map((result) => JSON.stringify(result.result ?? "")),
    [toolResultsMap],
  );

  return (
    <SessionRepositories
      sandboxState={data?.sandbox_state}
      repositories={data?.repositories}
      toolResultTexts={toolResultTexts}
      loading={isLoading}
      refreshing={isFetching}
      error={error ? ui("Could not read the repositories from the sandbox.") : undefined}
      onRefresh={() => void refetch()}
    />
  );
}
