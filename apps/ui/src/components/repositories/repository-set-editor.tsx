"use client";

// One editable set of GitHub repositories with read or write access each.
//
// Shared by the agent's Repositories settings (the usual set) and the new-chat
// form (the set one session works on), so both read the same way. Choices come
// from what the agent's GitHub identity can reach; without that list a name is
// typed, and GitHub still decides at checkout whether the agent can reach it.

import { useState } from "react";
import { Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Combobox } from "@/components/ui/combobox";
import { Input } from "@/components/ui/input";
import { SegmentedControl } from "@/components/ui/segmented-control";
import type { GitHubRepository } from "@/lib/api/agent-github";
import {
  isValidRepositoryName,
  normalizeRepositoryName,
  type RepositoryAccess,
  type RepositoryEntry,
} from "@/lib/repositories-config";
import { useUiTranslations } from "@/providers/locale-provider";

interface RepositorySetEditorProps {
  idPrefix: string;
  entries: RepositoryEntry[];
  onChange: (entries: RepositoryEntry[]) => void;
  /** Repositories the agent's GitHub identity reaches; undefined when not known. */
  available?: GitHubRepository[];
  /**
   * The most access `repo` may get, or `undefined` when it may not be in the
   * set. Leave unset where the set itself is the rule (agent settings).
   */
  maxAccess?: (repo: string) => RepositoryAccess | undefined;
  /** Offer a start branch per repository. */
  showBranch?: boolean;
  /** Whether repositories outside the current set may be added. */
  canAdd?: boolean;
  disabled?: boolean;
}

export function RepositorySetEditor({
  idPrefix,
  entries,
  onChange,
  available,
  maxAccess,
  showBranch = false,
  canAdd = true,
  disabled,
}: RepositorySetEditorProps) {
  const ui = useUiTranslations();
  const [typed, setTyped] = useState("");
  const [typedError, setTypedError] = useState<string | null>(null);

  const has = (repo: string) =>
    entries.some((entry) => entry.repo.toLowerCase() === repo.toLowerCase());
  const update = (index: number, patch: Partial<RepositoryEntry>) =>
    onChange(entries.map((entry, at) => (at === index ? { ...entry, ...patch } : entry)));

  const add = (input: string): string | null => {
    const repo = normalizeRepositoryName(input);
    if (!isValidRepositoryName(repo)) return ui("Enter a repository as owner/name.");
    if (has(repo)) return ui("This repository is already in the list.");
    const limit = maxAccess ? maxAccess(repo) : "write";
    if (!limit) return ui("This agent does not allow adding repositories.");
    onChange([...entries, { repo, access: "read" }]);
    return null;
  };

  const choices = (available ?? []).filter(
    (repo) => !has(repo.full_name) && (!maxAccess || maxAccess(repo.full_name)),
  );

  return (
    <div className="space-y-2">
      {entries.length > 0 && (
        <ul className="divide-y border">
          {entries.map((entry, index) => {
            const limit = maxAccess ? maxAccess(entry.repo) : "write";
            return (
              <li
                key={entry.repo}
                className="flex flex-wrap items-center gap-x-3 gap-y-1.5 px-2.5 py-2"
              >
                <span className="min-w-0 flex-1 basis-40 truncate font-mono text-[13px]">
                  {entry.repo}
                </span>
                {showBranch && (
                  <Input
                    aria-label={ui("Branch for {repo}", { repo: entry.repo })}
                    value={entry.ref ?? ""}
                    onChange={(event) => update(index, { ref: event.target.value || undefined })}
                    placeholder={ui("default branch")}
                    disabled={disabled}
                    className="h-8 w-36 font-mono text-[13px]"
                  />
                )}
                {limit === "write" ? (
                  <SegmentedControl
                    label={ui("Access for {repo}", { repo: entry.repo })}
                    value={entry.access}
                    onValueChange={(next) => update(index, { access: next as RepositoryAccess })}
                    options={[
                      { value: "read", label: ui("Read") },
                      { value: "write", label: ui("Write") },
                    ]}
                    disabled={disabled}
                  />
                ) : (
                  <span className="text-xs text-muted-foreground">
                    {ui("read only for this agent")}
                  </span>
                )}
                <Button
                  type="button"
                  variant="ghost"
                  size="icon-sm"
                  aria-label={ui("Remove {repo}", { repo: entry.repo })}
                  onClick={() => onChange(entries.filter((_, at) => at !== index))}
                  disabled={disabled}
                >
                  <X className="size-3.5" />
                </Button>
              </li>
            );
          })}
        </ul>
      )}
      {canAdd &&
        (choices.length > 0 ? (
          <Combobox
            id={`${idPrefix}-add`}
            options={choices.map((repo) => ({
              value: repo.full_name,
              label: repo.full_name,
              description: repo.private ? ui("private") : undefined,
            }))}
            value=""
            onValueChange={(repo) => {
              if (repo) add(repo);
            }}
            placeholder={ui("Add repository")}
            searchPlaceholder={ui("Search repositories")}
            ariaLabel={ui("Add repository")}
            clearable={false}
            disabled={disabled}
          />
        ) : available && available.length > 0 ? null : (
          <div className="space-y-1">
            <div className="flex gap-2">
              <Input
                id={`${idPrefix}-add`}
                aria-label={ui("Add repository")}
                value={typed}
                onChange={(event) => {
                  setTyped(event.target.value);
                  setTypedError(null);
                }}
                onKeyDown={(event) => {
                  if (event.key !== "Enter") return;
                  event.preventDefault();
                  const failure = add(typed);
                  setTypedError(failure);
                  if (!failure) setTyped("");
                }}
                placeholder={ui("owner/name")}
                disabled={disabled}
                className="h-8 font-mono text-[13px]"
              />
              <Button
                type="button"
                variant="outline"
                onClick={() => {
                  const failure = add(typed);
                  setTypedError(failure);
                  if (!failure) setTyped("");
                }}
                disabled={disabled || !typed.trim()}
              >
                <Plus className="size-3.5" />
                {ui("Add")}
              </Button>
            </div>
            {typedError && <p className="text-xs text-destructive">{typedError}</p>}
          </div>
        ))}
    </div>
  );
}
