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
import { useVolumes } from "@/hooks/use-volumes";
import type { Volume } from "@/lib/api/types";

// Mirrors crates/core/src/volume.rs: WorkspaceVolumesConfig wire shape.
export interface VolumeMountEntry {
  volume: string;
  path: string;
  mode?: "readonly" | "readwrite";
}

interface WorkspaceVolumesConfig {
  mounts?: VolumeMountEntry[];
}

interface WorkspaceVolumesConfigEditorProps {
  config: Record<string, unknown>;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}

const VOLUME_ID_PATTERN = /^vol_[0-9a-f]{32}$/;
const PATH_PREFIX = "/workspace";

// Validate the structural shape of a mount path. Mirrors
// validate_mount_config_shape in crates/core/src/volume.rs so that errors
// surface client-side with the same semantics as the server.
//
// Returns null for empty paths so freshly-added rows render without an
// immediate error; the empty path is still rejected at save time because the
// row is filtered from the overlap check and the server validates the final
// config.
export function validateMountPath(path: string): string | null {
  if (!path) return null;
  if (path !== PATH_PREFIX && !path.startsWith(`${PATH_PREFIX}/`)) {
    return `Path must be ${PATH_PREFIX} or start with ${PATH_PREFIX}/`;
  }
  if (path.includes("//")) return "Path must not contain '//'";
  if (path.includes("\0")) return "Path must not contain null bytes";
  if (path.split("/").some((segment) => segment === "..")) {
    return "Path must not contain '..'";
  }
  if (path.length > 1 && path.endsWith("/")) {
    return "Path must not end with a trailing slash";
  }
  return null;
}

// Returns true if `a` and `b` overlap: equal, or one is a path prefix of the
// other on a `/`-segment boundary. Mirrors mount_paths_overlap in
// crates/core/src/volume.rs but compares path segments so the result is
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

interface MountValidation {
  pathError?: string;
  volumeError?: string;
  overlapError?: string;
}

function validateMounts(mounts: VolumeMountEntry[]): MountValidation[] {
  const validations: MountValidation[] = mounts.map((mount) => {
    const validation: MountValidation = {};
    const pathErr = validateMountPath(mount.path);
    if (pathErr) validation.pathError = pathErr;
    if (!mount.volume) {
      validation.volumeError = "Volume is required";
    } else if (!VOLUME_ID_PATTERN.test(mount.volume)) {
      validation.volumeError = "Invalid volume ID";
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
        const message =
          a.path === b.path
            ? `Duplicate mount path '${a.path}'`
            : `Path '${a.path}' overlaps with '${b.path}'`;
        if (!validations[i].overlapError) validations[i].overlapError = message;
        if (!validations[j].overlapError) validations[j].overlapError = message;
      }
    }
  }

  return validations;
}

function readMounts(config: Record<string, unknown>): VolumeMountEntry[] {
  const typed = config as WorkspaceVolumesConfig;
  if (!Array.isArray(typed.mounts)) return [];
  return typed.mounts
    .filter((m): m is VolumeMountEntry => Boolean(m) && typeof m === "object")
    .map((m) => ({
      volume: String(m.volume ?? ""),
      path: String(m.path ?? ""),
      mode: m.mode === "readwrite" ? "readwrite" : "readonly",
    }));
}

function writeMounts(mounts: VolumeMountEntry[]): Record<string, unknown> {
  if (mounts.length === 0) return {};
  return {
    mounts: mounts.map((m) => ({
      volume: m.volume,
      path: m.path,
      mode: m.mode ?? "readonly",
    })),
  };
}

export function WorkspaceVolumesConfigEditor({
  config,
  onChange,
  disabled,
}: WorkspaceVolumesConfigEditorProps) {
  const mounts = readMounts(config);
  const { data: volumes, isLoading, error } = useVolumes({ includeArchived: false });

  const activeVolumes = useMemo<Volume[]>(
    () => (volumes ?? []).filter((v) => v.status === "active"),
    [volumes],
  );

  const volumeOptions = useMemo<ComboboxOption[]>(
    () =>
      activeVolumes.map((v) => ({
        value: v.id,
        label: v.name,
      })),
    [activeVolumes],
  );

  const validations = validateMounts(mounts);

  const updateMounts = (next: VolumeMountEntry[]) => {
    onChange(writeMounts(next));
  };

  const updateRow = (index: number, patch: Partial<VolumeMountEntry>) => {
    const next = mounts.map((mount, i) => (i === index ? { ...mount, ...patch } : mount));
    updateMounts(next);
  };

  const removeRow = (index: number) => {
    updateMounts(mounts.filter((_, i) => i !== index));
  };

  const addRow = () => {
    // Default to an empty path so the row does not start in an error state
    // (a placeholder shows "/workspace/research" as a hint instead).
    updateMounts([...mounts, { volume: "", path: "", mode: "readonly" }]);
  };

  return (
    <div className="space-y-3 pt-2">
      <div className="space-y-1">
        <Label className="text-xs font-normal text-muted-foreground">Mounts</Label>
        <p className="text-xs text-muted-foreground">
          Mount org volumes into <code>/workspace</code> for sessions using this capability.
        </p>
      </div>

      {error && <p className="text-xs text-destructive">Failed to load volumes. Try again.</p>}

      {!isLoading && activeVolumes.length === 0 && mounts.length === 0 && (
        <p className="text-xs text-muted-foreground">
          No active volumes found in this org. Create a volume first to add a mount.
        </p>
      )}

      {mounts.length > 0 && (
        <div className="space-y-2">
          {mounts.map((mount, index) => {
            const validation = validations[index];
            // If the configured volume is no longer in the active list, surface
            // a hint so the user can correct or replace it. Suppress while the
            // volume list is still loading or errored — `activeVolumes` is
            // empty in those states and would falsely flag valid IDs.
            const isUnknownVolume =
              !isLoading &&
              !error &&
              mount.volume.length > 0 &&
              VOLUME_ID_PATTERN.test(mount.volume) &&
              !activeVolumes.some((v) => v.id === mount.volume);
            const volumeError =
              validation.volumeError ??
              (isUnknownVolume ? "Volume is unavailable or archived" : undefined);

            return (
              <div
                key={index}
                className="space-y-1.5 rounded-md border border-border/60 p-2"
                data-testid="volume-mount-row"
              >
                <div className="flex items-start gap-2">
                  <div className="flex-1 space-y-1">
                    <Label className="text-xs font-normal text-muted-foreground">Volume</Label>
                    <Combobox
                      options={volumeOptions}
                      value={mount.volume}
                      onValueChange={(value) => updateRow(index, { volume: value })}
                      placeholder={isLoading ? "Loading..." : "Select a volume"}
                      searchPlaceholder="Search volumes..."
                      disabled={disabled || isLoading}
                    />
                    {volumeError && <p className="text-xs text-destructive">{volumeError}</p>}
                  </div>
                  <div className="flex-1 space-y-1">
                    <Label className="text-xs font-normal text-muted-foreground">Mount path</Label>
                    <Input
                      value={mount.path}
                      placeholder="/workspace/research"
                      disabled={disabled}
                      className="h-8 text-sm font-mono"
                      onChange={(event) => updateRow(index, { path: event.target.value })}
                    />
                    {(validation.pathError || validation.overlapError) && (
                      <p className="text-xs text-destructive">
                        {validation.pathError ?? validation.overlapError}
                      </p>
                    )}
                  </div>
                  <div className="w-32 space-y-1">
                    <Label className="text-xs font-normal text-muted-foreground">Mode</Label>
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
                        <SelectItem value="readonly">Read only</SelectItem>
                        <SelectItem value="readwrite">Read / write</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    aria-label="Remove mount"
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
        Add mount
      </Button>
    </div>
  );
}
