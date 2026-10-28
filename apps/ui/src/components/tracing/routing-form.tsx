"use client";
import { useEffect, useState } from "react";
import Link from "next/link";
import { HistorySheet } from "@/components/entity-actions/history-sheet";
import { z } from "zod";
import { useOrg } from "@/providers/org-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import { useTracingDestinations, useTracingPolicy, useRefreshTracing } from "@/hooks/use-tracing";
import {
  effectiveDestinations,
  saveTracingPolicy,
  type TracingPolicy,
  type RoutingMode,
} from "@/lib/api/tracing";
import { usePageFormDraft } from "@/components/chat-mode/use-page-form-draft";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select";

const schema = z
  .object({
    mode: z.enum(["inherit", "add", "replace", "off"]),
    destination_ids: z.array(z.string().uuid()).max(8),
  })
  .strict();
const modes: [RoutingMode, string][] = [
  ["inherit", "Use organization defaults"],
  ["add", "Add to organization defaults"],
  ["replace", "Use selected destinations"],
  ["off", "Turn off external tracing"],
];
export function TracingRoutingForm({ agentId }: { agentId?: string }) {
  const ui = useUiTranslations();
  const { currentOrg, hasRole } = useOrg();
  const canManage = hasRole("admin");
  const refresh = useRefreshTracing();
  const query = useTracingPolicy(agentId);
  const defaults = useTracingPolicy();
  const destinations = useTracingDestinations();
  const [draft, setDraft] = useState<TracingPolicy>({
    mode: agentId ? "inherit" : "replace",
    destination_ids: [],
  });
  const [baseline, setBaseline] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [history, setHistory] = useState(false);
  const [baselinePolicy, setBaselinePolicy] = useState<TracingPolicy | null>(null);
  const dirty = baselinePolicy !== null && JSON.stringify(draft) !== JSON.stringify(baselinePolicy);
  useEffect(() => {
    if (query.data && (baseline === null || (!dirty && baseline < query.data.revision))) {
      const next = {
        ...query.data.policy,
        mode: agentId ? query.data.policy.mode : ("replace" as const),
      };
      setDraft(next);
      setBaselinePolicy(next);
      setBaseline(query.data.revision);
    }
  }, [query.data, baseline, agentId, dirty]);
  const chat = usePageFormDraft({
    entityType: "tracing_policy",
    entityId: agentId ?? currentOrg?.public_id,
    label: ui("Tracing"),
    schema,
    values: draft,
    dirty,
    writable: canManage && !busy,
    baseline: String(query.data?.revision ?? 0),
    apply: (values) => {
      setDraft(values);
      setSaved(false);
    },
    enabled: !!query.data,
  });
  const conflict = baseline !== null && query.data?.revision !== baseline && dirty;
  function reset() {
    if (query.data) {
      const next = {
        ...query.data.policy,
        mode: agentId ? query.data.policy.mode : ("replace" as const),
      };
      setDraft(next);
      setBaselinePolicy(next);
      setBaseline(query.data.revision);
    }
    setError(null);
    chat.clear();
  }
  async function save() {
    if (baseline === null || conflict || chat.blocked) return;
    setBusy(true);
    setError(null);
    try {
      const result = await saveTracingPolicy({ policy: draft, revision: baseline }, agentId);
      setBaseline(result.revision);
      setBaselinePolicy(draft);
      setSaved(true);
      chat.clear();
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : ui("Could not save tracing"));
    } finally {
      setBusy(false);
    }
  }
  if (query.isLoading || destinations.isLoading || defaults.isLoading)
    return <p className="text-sm text-muted-foreground">{ui("Loading tracing…")}</p>;
  if (query.error || destinations.error || defaults.error)
    return (
      <div role="alert" className="space-y-2">
        <p>{ui("Could not load tracing")}</p>
        <Button
          variant="outline"
          onClick={() => {
            void query.refetch();
            void destinations.refetch();
            void defaults.refetch();
          }}
        >
          {ui("Retry")}
        </Button>
      </div>
    );
  const available = destinations.data ?? [];
  const inherited = defaults.data?.policy.destination_ids ?? [];
  const effective = effectiveDestinations(draft, inherited);
  const selectable = draft.mode === "add" || draft.mode === "replace";
  return (
    <div className="space-y-5">
      {chat.notice}
      {!canManage && (
        <p className="text-sm text-muted-foreground">
          {ui("Organization admins manage trace exports.")}
        </p>
      )}
      {agentId && (
        <div className="space-y-2">
          <Label htmlFor="tracing-mode">{ui("Routing")}</Label>
          <Select
            value={draft.mode}
            onValueChange={(value) => {
              setDraft({
                ...draft,
                mode: value as RoutingMode,
                destination_ids:
                  value === "inherit" || value === "off" ? [] : draft.destination_ids,
              });
              setSaved(false);
            }}
            disabled={!canManage || busy}
          >
            <SelectTrigger id="tracing-mode" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {modes.map(([mode, label]) => (
                <SelectItem key={mode} value={mode}>
                  {ui(label)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}
      {selectable && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium">
            {ui(agentId ? "Selected destinations" : "Organization defaults")}
          </h3>
          {available.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              {ui("Connect a destination before selecting defaults.")}
            </p>
          ) : (
            available.map((d) => (
              <label key={d.id} className="flex items-start gap-3 border p-3">
                <Checkbox
                  disabled={
                    !canManage ||
                    busy ||
                    (draft.destination_ids.length >= 8 && !draft.destination_ids.includes(d.id))
                  }
                  checked={draft.destination_ids.includes(d.id)}
                  onCheckedChange={(checked) => {
                    setDraft({
                      ...draft,
                      destination_ids: checked
                        ? [...draft.destination_ids, d.id]
                        : draft.destination_ids.filter((id) => id !== d.id),
                    });
                    setSaved(false);
                  }}
                />
                <span className="min-w-0 text-sm">
                  <span className="font-medium">{d.config.name}</span>
                  <span className="mt-1 block text-xs text-muted-foreground">
                    {ui(d.config.capture_content ? "Messages and tools" : "Metadata only")}
                    {!d.config.enabled && ` · ${ui("Paused")}`}
                  </span>
                </span>
              </label>
            ))
          )}
        </div>
      )}
      {agentId && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium">{ui("Effective destinations")}</h3>
          {effective.length === 0 ? (
            <p className="text-sm text-muted-foreground">{ui("No external destinations.")}</p>
          ) : (
            effective.map((id) => {
              const d = available.find((d) => d.id === id);
              return (
                <div key={id} className="flex items-center justify-between gap-2 border p-3">
                  <span className="text-sm">
                    {d?.config.name ?? ui("Destination unavailable")}
                    {d && !d.config.enabled && (
                      <span className="ml-2 text-xs text-muted-foreground">{ui("Paused")}</span>
                    )}
                  </span>
                  <Badge variant="outline">
                    {ui(
                      inherited.includes(id) && draft.mode !== "replace"
                        ? "Inherited"
                        : "Agent selection",
                    )}
                  </Badge>
                </div>
              );
            })
          )}
        </div>
      )}
      <p className="text-xs text-muted-foreground">
        {ui("Changes apply to new turns in existing and new sessions. Personal Chat is excluded.")}
      </p>
      {(conflict || chat.blocked) && (
        <p role="alert" className="text-sm text-destructive">
          {ui("Routing changed elsewhere. Discard this draft and reload before saving.")}
        </p>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {canManage && (
        <div className="flex flex-wrap items-center gap-2">
          <Button onClick={() => void save()} disabled={!dirty || busy || conflict || chat.blocked}>
            {ui(busy ? "Saving…" : "Save changes")}
          </Button>
          <Button variant="outline" onClick={reset} disabled={busy}>
            {ui("Discard")}
          </Button>
          <Button variant="ghost" onClick={() => setHistory(true)}>
            {ui("History")}
          </Button>
          {saved && (
            <span role="status" className="text-xs text-success">
              {ui("Saved")}
            </span>
          )}
        </div>
      )}
      {history && (
        <HistorySheet
          entityRef={agentId ?? currentOrg!.public_id}
          kind="tracing_policy"
          entityName={ui("Tracing")}
          canRestore={canManage && !dirty}
          open={history}
          onOpenChange={setHistory}
        />
      )}
      {agentId && (
        <Link
          className="inline-block text-xs text-muted-foreground underline"
          href="/settings/tracing"
        >
          {ui("Manage organization destinations")}
        </Link>
      )}
    </div>
  );
}
