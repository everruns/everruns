"use client";

import { useMemo } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Combobox, type ComboboxOption } from "@/components/ui/combobox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useMemories } from "@/hooks/use-memory";
import type { Memory } from "@/lib/api/types";
import type { MessageKey } from "@/lib/i18n";
import { useLocale } from "@/providers/locale-provider";

// Mirrors crates/core/src/memory.rs: MemoryConfig wire shape.
export interface MemoryMountEntry {
  memory: string;
  path: string;
  mode?: "readonly" | "readwrite";
}

interface MemoryConfig {
  mounts?: MemoryMountEntry[];
}

interface MemoryConfigEditorProps {
  config: unknown;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}

const VOLUME_ID_PATTERN = /^mem_[0-9a-f]{32}$/;
const PATH_PREFIX = "/workspace";

// Validate the structural shape of a mount path. Mirrors
// validate_mount_config_shape in crates/core/src/memory.rs so that errors
// surface client-side with the same semantics as the server. Returns a
// message key so the caller renders it in the active locale.
//
// Returns null for empty paths so freshly-added rows render without an
// immediate error; the empty path is still rejected at save time because the
// row is filtered from the overlap check and the server validates the final
// config.
export function validateMountPath(path: string): MessageKey | null {
  if (!path) return null;
  if (path !== PATH_PREFIX && !path.startsWith(`${PATH_PREFIX}/`)) {
    return "mount_path_must_start";
  }
  if (path.includes("//")) return "mount_path_double_slash";
  if (path.includes("\0")) return "mount_path_null";
  if (path.split("/").some((segment) => segment === "..")) {
    return "mount_path_dotdot";
  }
  if (path.length > 1 && path.endsWith("/")) {
    return "mount_path_trailing_slash";
  }
  return null;
}

// Returns true if `a` and `b` overlap: equal, or one is a path prefix of the
// other on a `/`-segment boundary. Mirrors mount_paths_overlap in
// crates/core/src/memory.rs but compares path segments so the result is
// independent of multi-byte character widths in non-ASCII paths.
export function pathsOverlap(a: string, b: string): boolean {
  if (a === b) return true;
  const segA = a.split("/");
  const segB = b.split("/");
  const [shorter, longer] = segA.length < segB.length ? [segA, segB] : [segB, segA];
  for (let i = 0; i < shorter.length; i++) {
    if (shorter[i] !== longer[i]) return false;
  }
  return true;
}

interface LocalizedError {
  key: MessageKey;
  values?: Record<string, string | number>;
}

interface MountValidation {
  pathError?: LocalizedError;
  memoryError?: LocalizedError;
  overlapError?: LocalizedError;
}

function validateMounts(mounts: MemoryMountEntry[]): MountValidation[] {
  const validations: MountValidation[] = mounts.map((mount) => {
    const validation: MountValidation = {};
    const pathErr = validateMountPath(mount.path);
    if (pathErr) validation.pathError = { key: pathErr };
    if (!mount.memory) {
      validation.memoryError = { key: "memory_required" };
    } else if (!VOLUME_ID_PATTERN.test(mount.memory)) {
      validation.memoryError = { key: "memory_invalid_id" };
    }
    return validation;
  });

  // Detect duplicate / overlapping paths between rows. Mark both rows so the
  // user sees the conflict on either side regardless of row order.
  for (let i = 0; i < mounts.length; i++) {
    const a = mounts[i];
    if (!a.path || validations[i].pathError) continue;
    for (let j = i + 1; j < mounts.length; j++) {
      const b = mounts[j];
      if (!b.path || validations[j].pathError) continue;
      if (pathsOverlap(a.path, b.path)) {
        // Each row leads with its own path so the error reads naturally
        // under that row's input.
        const [messageA, messageB]: [LocalizedError, LocalizedError] =
          a.path === b.path
            ? [
                { key: "mount_path_duplicate", values: { value: a.path } },
                { key: "mount_path_duplicate", values: { value: b.path } },
              ]
            : [
                { key: "mount_path_overlap", values: { value: a.path, other: b.path } },
                { key: "mount_path_overlap", values: { value: b.path, other: a.path } },
              ];
        if (!validations[i].overlapError) validations[i].overlapError = messageA;
        if (!validations[j].overlapError) validations[j].overlapError = messageB;
      }
    }
  }

  return validations;
}

function readMounts(config: unknown): MemoryMountEntry[] {
  if (!config || typeof config !== "object") return [];
  const typed = config as MemoryConfig;
  if (!Array.isArray(typed.mounts)) return [];
  return typed.mounts
    .filter((m): m is MemoryMountEntry => Boolean(m) && typeof m === "object")
    .map((m) => ({
      memory: String(m.memory ?? ""),
      path: String(m.path ?? ""),
      mode: m.mode === "readwrite" ? "readwrite" : "readonly",
    }));
}

function writeMounts(mounts: MemoryMountEntry[]): Record<string, unknown> {
  if (mounts.length === 0) return {};
  return {
    mounts: mounts.map((m) => ({
      memory: m.memory,
      path: m.path,
      mode: m.mode ?? "readonly",
    })),
  };
}

export function MemoryConfigEditor({ config, onChange, disabled }: MemoryConfigEditorProps) {
  const { t } = useLocale();
  const mounts = readMounts(config);
  const { data: memory, isLoading, error } = useMemories({ includeArchived: false });

  const activeMemories = useMemo<Memory[]>(
    () => (memory ?? []).filter((v) => v.status === "active"),
    [memory],
  );

  const memoryOptions = useMemo<ComboboxOption[]>(
    () =>
      activeMemories.map((v) => ({
        value: v.id,
        label: v.name,
      })),
    [activeMemories],
  );

  const validations = validateMounts(mounts);

  const updateMounts = (next: MemoryMountEntry[]) => {
    onChange(writeMounts(next));
  };

  const updateRow = (index: number, patch: Partial<MemoryMountEntry>) => {
    const next = mounts.map((mount, i) => (i === index ? { ...mount, ...patch } : mount));
    updateMounts(next);
  };

  const removeRow = (index: number) => {
    updateMounts(mounts.filter((_, i) => i !== index));
  };

  const addRow = () => {
    // Default to an empty path so the row does not start in an error state
    // (a placeholder shows "/workspace/research" as a hint instead).
    updateMounts([...mounts, { memory: "", path: "", mode: "readonly" }]);
  };

  return (
    <div className="space-y-3 pt-2">
      <div className="space-y-1">
        <Label className="text-xs font-normal text-muted-foreground">{t("memory_mounts")}</Label>
        <p className="text-xs text-muted-foreground">{t("memory_mounts_help")}</p>
      </div>

      {error && <p className="text-xs text-destructive">{t("memory_load_failed")}</p>}

      {!isLoading && activeMemories.length === 0 && mounts.length === 0 && (
        <p className="text-xs text-muted-foreground">{t("memory_none_active")}</p>
      )}

      {mounts.length > 0 && (
        <div className="space-y-2">
          {mounts.map((mount, index) => {
            const validation = validations[index];
            // If the configured memory is no longer in the active list, surface
            // a hint so the user can correct or replace it. Suppress while the
            // memory list is still loading or errored — `activeMemories` is
            // empty in those states and would falsely flag valid IDs.
            const isUnknownMemory =
              !isLoading &&
              !error &&
              mount.memory.length > 0 &&
              VOLUME_ID_PATTERN.test(mount.memory) &&
              !activeMemories.some((v) => v.id === mount.memory);
            const memoryError =
              validation.memoryError ??
              (isUnknownMemory ? ({ key: "memory_unavailable" } as LocalizedError) : undefined);

            return (
              <div
                key={index}
                className="space-y-1.5 rounded-md border border-border/60 p-2"
                data-testid="memory-mount-row"
              >
                <div className="flex items-start gap-2">
                  <div className="flex-1 space-y-1">
                    <Label className="text-xs font-normal text-muted-foreground">
                      {t("memory_label")}
                    </Label>
                    <Combobox
                      options={memoryOptions}
                      value={mount.memory}
                      onValueChange={(value) => updateRow(index, { memory: value })}
                      placeholder={isLoading ? t("memory_loading") : t("memory_select")}
                      searchPlaceholder={t("memory_search")}
                      disabled={disabled || isLoading}
                    />
                    {memoryError && (
                      <p className="text-xs text-destructive">
                        {t(memoryError.key, memoryError.values)}
                      </p>
                    )}
                  </div>
                  <div className="flex-1 space-y-1">
                    <Label className="text-xs font-normal text-muted-foreground">
                      {t("mount_path_label")}
                    </Label>
                    <Input
                      value={mount.path}
                      placeholder="/workspace/research"
                      disabled={disabled}
                      className="h-8 text-sm font-mono"
                      onChange={(event) => updateRow(index, { path: event.target.value })}
                    />
                    {(validation.pathError || validation.overlapError) && (
                      <p className="text-xs text-destructive">
                        {(() => {
                          const err = validation.pathError ?? validation.overlapError;
                          return err ? t(err.key, err.values) : null;
                        })()}
                      </p>
                    )}
                  </div>
                  <div className="w-32 space-y-1">
                    <Label className="text-xs font-normal text-muted-foreground">
                      {t("mount_mode_label")}
                    </Label>
                    <Select
                      value={mount.mode ?? "readonly"}
                      onValueChange={(value) =>
                        updateRow(index, { mode: value as "readonly" | "readwrite" })
                      }
                      disabled={disabled}
                    >
                      <SelectTrigger className="h-8 text-sm">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="readonly">{t("mount_mode_readonly")}</SelectItem>
                        <SelectItem value="readwrite">{t("mount_mode_readwrite")}</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    aria-label={t("mount_remove")}
                    disabled={disabled}
                    className="mt-5 h-8 w-8 text-muted-foreground hover:text-destructive"
                    onClick={() => removeRow(index)}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      <Button type="button" variant="outline" size="sm" disabled={disabled} onClick={addRow}>
        <Plus className="mr-1 h-3.5 w-3.5" />
        {t("mount_add")}
      </Button>
    </div>
  );
}
