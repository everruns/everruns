"use client";

// Settings for the `agent_memory` capability. Mirrors AgentMemoryConfig in
// crates/contracts/src/runtime/memory.rs: one switch per store (off, read,
// read_write; parent is read-only), named memories to attach, and whether the
// memory card is shown to the agent. Missing keys keep the server defaults.

import { useMemo } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Combobox, type ComboboxOption } from "@/components/ui/combobox";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useMemoryStores } from "@/hooks/use-memory";
import { useUiTranslations } from "@/providers/locale-provider";

export type MemoryAccess = "off" | "read" | "read_write";

export interface MemoryAttachment {
  memory: string;
  mode: MemoryAccess;
}

type StoreKey = "agent" | "team" | "parent" | "user" | "org";

export const MEMORY_STORE_DEFAULTS: Record<StoreKey, MemoryAccess> = {
  agent: "read_write",
  team: "read_write",
  parent: "read",
  user: "read_write",
  org: "read",
};

const STORES: { key: StoreKey; label: string; help: string }[] = [
  { key: "agent", label: "Agent", help: "This agent's own memory, shared by all its sessions." },
  {
    key: "team",
    label: "Team",
    help: "Shared by a coordinator and every thread or agent it starts.",
  },
  {
    key: "parent",
    label: "Parent",
    help: "The memory of the agent that started this one. Read only.",
  },
  {
    key: "user",
    label: "Person",
    help: "Private to the person using the agent; only their sessions read it.",
  },
  { key: "org", label: "Organization", help: "The organization's memory, read by every agent." },
];

function readAccess(value: unknown, fallback: MemoryAccess): MemoryAccess {
  return value === "off" || value === "read" || value === "read_write" ? value : fallback;
}

/** The attachments in `config`, ignoring malformed rows. */
export function readAttachments(config: Record<string, unknown>): MemoryAttachment[] {
  const attach = Array.isArray(config.attach) ? config.attach : [];
  return attach.flatMap((row) => {
    if (!row || typeof row !== "object") return [];
    const memory = (row as Record<string, unknown>).memory;
    if (typeof memory !== "string") return [];
    return [{ memory, mode: readAccess((row as Record<string, unknown>).mode, "read") }];
  });
}

interface MemoryConfigEditorProps {
  config: unknown;
  onChange: (config: Record<string, unknown>) => void;
  disabled?: boolean;
}

export function MemoryConfigEditor({ config, onChange, disabled }: MemoryConfigEditorProps) {
  const ui = useUiTranslations();
  const current = useMemo<Record<string, unknown>>(
    () => (config && typeof config === "object" ? (config as Record<string, unknown>) : {}),
    [config],
  );
  const attachments = readAttachments(current);
  const card = current.card !== false;
  const { data: stores, isLoading } = useMemoryStores();

  const namedOptions = useMemo<ComboboxOption[]>(
    () =>
      (stores ?? [])
        .filter((store) => store.scope === "org" && store.store !== "org")
        .map((store) => ({ value: store.store, label: store.store })),
    [stores],
  );

  const set = (patch: Record<string, unknown>) => onChange({ ...current, ...patch });
  const setAttachments = (next: MemoryAttachment[]) => set({ attach: next });

  return (
    <div className="space-y-4 pt-2">
      <div className="space-y-2">
        <Label className="text-xs font-normal text-muted-foreground">{ui("Stores")}</Label>
        <div className="divide-y border">
          {STORES.map((store) => {
            const access = readAccess(current[store.key], MEMORY_STORE_DEFAULTS[store.key]);
            return (
              <div key={store.key} className="flex items-center gap-3 p-2">
                <div className="min-w-0 flex-1">
                  <div className="text-sm">
                    {ui(store.label)}{" "}
                    <span className="font-mono text-xs text-muted-foreground">
                      /memory/{store.key}
                    </span>
                  </div>
                  <p className="text-xs text-muted-foreground">{ui(store.help)}</p>
                </div>
                <Select
                  value={access}
                  disabled={disabled}
                  onValueChange={(value) => set({ [store.key]: value })}
                >
                  <SelectTrigger
                    className="h-8 w-36 text-sm"
                    aria-label={`${ui(store.label)} ${ui("access")}`}
                  >
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="off">{ui("Off")}</SelectItem>
                    <SelectItem value="read">{ui("Read")}</SelectItem>
                    {store.key !== "parent" && (
                      <SelectItem value="read_write">{ui("Read and write")}</SelectItem>
                    )}
                  </SelectContent>
                </Select>
              </div>
            );
          })}
        </div>
      </div>

      <div className="space-y-2">
        <Label className="text-xs font-normal text-muted-foreground">{ui("Named memories")}</Label>
        <p className="text-xs text-muted-foreground">
          {ui("Shared stores this agent reads at /memory/<name>, alongside other agents.")}
        </p>
        {attachments.map((attachment, index) => (
          <div key={index} className="flex items-center gap-2">
            <Combobox
              className="flex-1"
              options={namedOptions}
              value={attachment.memory}
              loading={isLoading}
              disabled={disabled}
              ariaLabel={ui("Named memory")}
              placeholder={ui("Select a memory")}
              searchPlaceholder={ui("Search memory")}
              onValueChange={(value) =>
                setAttachments(
                  attachments.map((row, i) => (i === index ? { ...row, memory: value } : row)),
                )
              }
            />
            <Select
              value={attachment.mode === "read_write" ? "read_write" : "read"}
              disabled={disabled}
              onValueChange={(value) =>
                setAttachments(
                  attachments.map((row, i) =>
                    i === index ? { ...row, mode: value as MemoryAccess } : row,
                  ),
                )
              }
            >
              <SelectTrigger className="h-8 w-36 text-sm" aria-label={ui("Access")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="read">{ui("Read")}</SelectItem>
                <SelectItem value="read_write">{ui("Read and write")}</SelectItem>
              </SelectContent>
            </Select>
            <Button
              type="button"
              variant="ghost"
              size="icon"
              disabled={disabled}
              aria-label={ui("Remove")}
              onClick={() => setAttachments(attachments.filter((_, i) => i !== index))}
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
        ))}
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={disabled}
          onClick={() => setAttachments([...attachments, { memory: "", mode: "read" }])}
        >
          <Plus className="h-4 w-4" />
          {ui("Attach a named memory")}
        </Button>
      </div>

      <div className="flex items-center justify-between gap-3">
        <div>
          <Label htmlFor="memory-card" className="text-sm font-normal">
            {ui("Memory card")}
          </Label>
          <p className="text-xs text-muted-foreground">
            {ui(
              "Keep the agent's open blocked items, decisions and rules in its context as they change.",
            )}
          </p>
        </div>
        <Switch
          id="memory-card"
          checked={card}
          disabled={disabled}
          onCheckedChange={(checked) => set({ card: checked })}
        />
      </div>
    </div>
  );
}
