"use client";

import type { MonitoringActorPage, MonitoringActorDetail } from "@/lib/api/types";
import { Button } from "@/components/ui/button";
import { EntityIdentity } from "@/components/ui/entity-identity";
import { shortenId } from "@/lib/utils";
import { duration, number, phases } from "./model";

export function ActorTable({
  page,
  selected,
  onSelect,
  offset,
  onPage,
  disabled,
}: {
  page?: MonitoringActorPage;
  selected: string | null;
  onSelect: (id: string) => void;
  offset: number;
  onPage: (offset: number) => void;
  disabled: boolean;
}) {
  return (
    <section className="border bg-card" data-testid="monitoring-actors">
      <div className="flex items-center justify-between border-b p-4">
        <h2 className="text-sm font-semibold">Session actors</h2>
        <span className="text-xs text-muted-foreground">
          Expired leases and oldest runnable actors first
        </span>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-left text-xs">
          <thead className="bg-muted/30 text-muted-foreground">
            <tr>
              {["Session", "Phase", "Lease", "Holder", "Fence", "Runnable age"].map((label) => (
                <th key={label} className="p-3 font-medium">
                  {label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {page?.actors.map((actor) => (
              <tr
                key={actor.session_id}
                className={selected === actor.session_id ? "bg-accent/10" : "hover:bg-muted/30"}
              >
                <td className="border-t p-3">
                  <button
                    type="button"
                    title={actor.session_id}
                    aria-label={`Inspect actor ${actor.session_id}`}
                    className="font-mono underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-ring"
                    onClick={() => onSelect(actor.session_id)}
                    disabled={disabled}
                  >
                    session_…{shortenId(actor.session_id)}
                  </button>
                </td>
                <td className="border-t p-3">
                  {phases.find((p) => p.key === actor.phase)?.label ?? actor.phase}
                </td>
                <td
                  className={`border-t p-3 ${actor.lease_state === "expired" ? "text-warning" : ""}`}
                >
                  {actor.lease_state}
                </td>
                <td
                  className="max-w-48 truncate border-t p-3 font-mono"
                  title={actor.holder ?? undefined}
                >
                  {actor.holder ?? "Unassigned"}
                </td>
                <td className="border-t p-3 font-mono">{actor.fence ?? "—"}</td>
                <td className="border-t p-3 tabular-nums">
                  {actor.runnable_age_ms == null ? "—" : duration(actor.runnable_age_ms)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {page?.actors.length === 0 && (
        <p className="p-12 text-center text-sm text-muted-foreground">
          No actors match these filters.
        </p>
      )}
      {!page && (
        <p className="p-12 text-center text-sm text-muted-foreground">Loading actor ownership…</p>
      )}
      <div className="flex items-center justify-between border-t p-3 text-xs text-muted-foreground">
        <span>
          {offset + 1}–{offset + (page?.actors.length ?? 0)}
        </span>
        <div className="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={disabled || offset === 0}
            onClick={() => onPage(Math.max(0, offset - 200))}
          >
            Previous
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={disabled || page?.next_offset == null}
            onClick={() => onPage(page!.next_offset!)}
          >
            Next
          </Button>
        </div>
      </div>
    </section>
  );
}

export function ActorDetail({
  detail,
  serverNow,
  stale,
}: {
  detail: MonitoringActorDetail;
  serverNow: number;
  stale: boolean;
}) {
  const actor = detail.actor;
  const remaining = actor.lease_expires_at
    ? Math.max(0, Date.parse(actor.lease_expires_at) - serverNow)
    : null;
  const leaseState =
    actor.lease_state === "held" && remaining === 0 && !stale
      ? "Expiry deadline passed; awaiting observation"
      : actor.lease_state;
  return (
    <div className="border bg-card p-4 text-xs" data-testid="monitoring-actor-detail">
      <h2 className="mb-2 text-sm font-semibold">Session actor</h2>
      <EntityIdentity value={actor.session_id}>
        <span className="font-mono">session_…{shortenId(actor.session_id)}</span>
      </EntityIdentity>
      <p className="mt-2 break-all text-muted-foreground">{actor.org_id}</p>
      <dl className="mt-4 space-y-2 border-t pt-4">
        <Detail
          label="Phase"
          value={phases.find((p) => p.key === actor.phase)?.label ?? actor.phase}
        />
        <Detail label="Session state" value={actor.session_status} />
        <Detail label="Lease state" value={leaseState} />
        <Detail
          label={actor.lease_state === "expired" ? "Previous holder" : "Holder"}
          value={actor.holder ?? "Unassigned"}
        />
        <Detail label="Fence" value={actor.fence == null ? "Unavailable" : String(actor.fence)} />
        <Detail
          label="Lease remaining"
          value={stale ? "Snapshot stale" : remaining == null ? "Unleased" : duration(remaining)}
        />
        <Detail label="Lease TTL" value={duration(actor.lease_ttl_ms)} />
        <Detail label="Expires (UTC)" value={time(actor.lease_expires_at)} />
        <Detail label="Renewed (UTC)" value={time(actor.lease_renewed_at)} />
        <Detail label="Runnable age" value={duration(actor.runnable_age_ms)} />
      </dl>
      <h3 className="mt-5 border-t pt-4 font-semibold">Durable progress</h3>
      <dl className="mt-3 space-y-2">
        <Detail label="inbox_seq" value={String(actor.inbox_seq ?? "Unavailable")} />
        <Detail label="consumed_seq" value={String(actor.consumed_seq ?? "Unavailable")} />
        <Detail label="next_input_seq" value={String(actor.next_input_seq ?? "Unavailable")} />
        <Detail
          label="Attempts since release"
          value={String(actor.attempts_since_release ?? "Unavailable")}
        />
        <Detail
          label="Cancel requested"
          value={actor.cancel_requested_at ? time(actor.cancel_requested_at) : "No"}
        />
      </dl>
      <p className="mt-3 text-[11px] leading-relaxed text-muted-foreground">
        Cursors are event sequences; their difference is not a count of pending messages. Expiry
        does not establish worker death.
      </p>
      <h3 className="mt-5 border-t pt-4 font-semibold">Ownership history</h3>
      {detail.ownership_history.length === 0 ? (
        <p className="mt-2 text-muted-foreground">No ownership changes recorded.</p>
      ) : (
        <ol className="mt-3 max-h-64 space-y-3 overflow-y-auto">
          {detail.ownership_history.map((event, i) => (
            <li key={`${event.occurred_at}-${i}`} className="border-l-2 border-accent pl-3">
              <div className="flex justify-between gap-2">
                <span className="font-medium">{event.kind}</span>
                <span className="text-muted-foreground">{time(event.occurred_at)}</span>
              </div>
              <p className="mt-1 break-all font-mono text-[10px]">
                {event.previous_holder ? `${event.previous_holder} → ` : ""}
                {event.holder}
              </p>
              <p className="mt-1 text-muted-foreground">
                Fence {event.fence}
                {event.kind === "reacquired" && " · resumed ownership"}
              </p>
            </li>
          ))}
        </ol>
      )}
      <h3 className="mt-5 border-t pt-4 font-semibold">Latest trace steps</h3>
      <p className="mt-2 text-[11px] text-muted-foreground">
        Up to 100 projected steps. Running model calls appear after generation completes.
      </p>
      <ol className="mt-3 max-h-56 space-y-2 overflow-y-auto">
        {detail.steps.map((step) => (
          <li key={`${step.turn_no}-${step.step_no}`} className="flex justify-between gap-3">
            <span>
              Turn {step.turn_no} · {step.kind} {step.step_no}
            </span>
            <span className="tabular-nums text-muted-foreground">
              {step.duration_ms == null ? step.status : duration(step.duration_ms)}
            </span>
          </li>
        ))}
      </ol>
      {!detail.steps.length && (
        <p className="mt-2 text-muted-foreground">No trace steps projected yet.</p>
      )}
      <p className="mt-4 border-t pt-3 text-[10px] text-muted-foreground">
        Snapshot {time(detail.generated_at)} UTC · {number(detail.ownership_history.length)}{" "}
        ownership events
      </p>
    </div>
  );
}
function time(value?: string | null) {
  return value ? new Date(value).toISOString().slice(11, 23) : "Unavailable";
}
function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-3">
      <dt className="shrink-0 text-muted-foreground">{label}</dt>
      <dd className="break-all text-right font-mono text-[11px]">{value}</dd>
    </div>
  );
}
