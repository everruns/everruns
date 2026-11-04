"use client";

// "Fix 3 Slack apps": when several open issues offer the same fixer, walk
// through them one at a time. Slack approves each app separately, so this never
// opens several consent screens at once. The queue lives in session storage so
// it survives the round trip through Slack, and an item counts as done once
// the issue stops appearing as pending.
import { useEffect, useMemo, useState } from "react";
import { CheckCircle2, ExternalLink, Wand2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useUiTranslations } from "@/providers/locale-provider";
import { useStartFix } from "@/components/health/health-fixer";
import type { HealthIssue } from "@/lib/api/types";

const STORAGE_KEY = "everruns.health-fix-queue";

interface Queue {
  kind: string;
  items: { id: string; name: string }[];
}

function readQueue(): Queue | null {
  try {
    const raw = window.sessionStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as Queue) : null;
  } catch {
    return null;
  }
}
function writeQueue(queue: Queue | null) {
  try {
    if (queue) window.sessionStorage.setItem(STORAGE_KEY, JSON.stringify(queue));
    else window.sessionStorage.removeItem(STORAGE_KEY);
  } catch {
    // Without storage the walk-through still works within this page.
  }
}

/** Open issues grouped by a fixer kind they can all run now. */
export function fixGroups(issues: HealthIssue[]) {
  const groups = new Map<string, HealthIssue[]>();
  for (const issue of issues) {
    for (const fixer of issue.fixers ?? []) {
      if (fixer.unavailable_reason) continue;
      groups.set(fixer.kind, [...(groups.get(fixer.kind) ?? []), issue]);
    }
  }
  return [...groups.entries()].filter(([, members]) => members.length > 1);
}

function noun(kind: string, count: number, ui: ReturnType<typeof useUiTranslations>) {
  if (kind.startsWith("slack.")) return ui("Fix {value0} Slack apps", { value0: String(count) });
  return ui("Fix {value0} issues", { value0: String(count) });
}

export function HealthFixQueue({ issues }: { issues: HealthIssue[] }) {
  const ui = useUiTranslations();
  const [queue, setQueue] = useState<Queue | null>(null);
  useEffect(() => setQueue(readQueue()), []);
  const pending = useMemo(() => new Map(issues.map((i) => [i.id, i])), [issues]);
  const next = queue?.items.find((item) => pending.has(item.id));
  const { start, state, pending: running } = useStartFix(next?.id ?? "");

  // Everything in the queue is resolved: the walk-through is over.
  useEffect(() => {
    if (queue && !next) {
      writeQueue(null);
    }
  }, [queue, next]);

  const begin = (kind: string, members: HealthIssue[]) => {
    const fresh: Queue = {
      kind,
      items: members.map((m) => ({ id: m.id, name: m.agent_name ?? ui("Organization") })),
    };
    writeQueue(fresh);
    setQueue(fresh);
    void start(kind, fresh.items[0].id);
  };
  const stop = () => {
    writeQueue(null);
    setQueue(null);
  };

  if (queue && next) {
    return (
      <section
        aria-label={ui("Fixing several issues")}
        className="flex flex-wrap items-start justify-between gap-4 border border-success/40 bg-success/5 p-4"
      >
        <div className="space-y-2">
          <p className="text-sm font-medium">
            {queue.kind.startsWith("slack.")
              ? ui("Fixing {value0} Slack apps", { value0: String(queue.items.length) })
              : ui("Fixing {value0} issues", { value0: String(queue.items.length) })}
          </p>
          <ol className="space-y-1 text-sm">
            {queue.items.map((item, index) => {
              const done = !pending.has(item.id);
              const current = item.id === next.id;
              return (
                <li key={item.id} className="flex items-center gap-2">
                  {done ? (
                    <CheckCircle2 className="size-4 text-success" aria-hidden />
                  ) : (
                    <span
                      className={`inline-flex size-4 items-center justify-center rounded-full border text-[10px] ${current ? "border-warning text-warning" : "border-border"}`}
                    >
                      {index + 1}
                    </span>
                  )}
                  <span>{item.name}</span>
                  <span className="text-xs text-muted-foreground">
                    {done ? ui("· fixed") : current ? ui("· next") : ""}
                  </span>
                </li>
              );
            })}
          </ol>
          {state.phase === "away" && (
            <p role="status" className="text-sm">
              {ui(
                "Finish in the Slack tab, then come back here. Everruns checks the app again when you return.",
              )}
            </p>
          )}
          {state.phase === "error" && (
            <p role="alert" className="text-sm text-destructive">
              {state.message}
            </p>
          )}
        </div>
        <div className="flex gap-2">
          <Button
            type="button"
            size="sm"
            disabled={running}
            onClick={() => void start(queue.kind, next.id)}
          >
            {ui("Continue with {value0}", { value0: next.name })}
            <ExternalLink className="size-3" aria-hidden />
          </Button>
          <Button type="button" size="sm" variant="ghost" onClick={stop}>
            {ui("Stop")}
          </Button>
        </div>
      </section>
    );
  }

  const groups = fixGroups(issues);
  if (groups.length === 0) return null;
  return (
    <>
      {groups.map(([kind, members]) => (
        <section
          key={kind}
          aria-label={ui("Several issues share one fix")}
          className="flex flex-wrap items-center justify-between gap-4 border border-warning/40 bg-warning/5 p-4"
        >
          <p className="text-sm">
            <span className="font-medium">
              {kind.startsWith("slack.")
                ? ui("{value0} Slack apps need the same fix.", { value0: String(members.length) })
                : ui("{value0} issues need the same fix.", { value0: String(members.length) })}
            </span>{" "}
            <span className="text-muted-foreground">
              {ui("Fix them one after another; each one may ask for its own approval.")}
            </span>
          </p>
          <Button type="button" size="sm" onClick={() => begin(kind, members)}>
            <Wand2 className="size-3" aria-hidden />
            {noun(kind, members.length, ui)}
          </Button>
        </section>
      ))}
    </>
  );
}
