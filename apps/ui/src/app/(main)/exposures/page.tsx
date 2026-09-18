"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { Globe, ShieldAlert, Radio } from "lucide-react";
import { usePageTitle } from "@/hooks";
import { useResumeAgentExposures, useSuspendAgentExposures } from "@/hooks/use-agents";
import {
  exposureStateLabel,
  useOrgExposures,
  type ExposureState,
  type OrgExposure,
} from "@/hooks/use-org-exposures";
import { usePolicies } from "@/hooks/use-policies";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  EmptyState,
  StatCard,
  StatGrid,
} from "@/components/layout";
import { getChannelTypeDisplayName } from "@/lib/app-channels";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { pluralize } from "@/lib/formatting";

/// Rows an operator scans for during an incident come first, then everything
/// else by how recently it was invoked. Sorting by agent would bury the one
/// row that matters among the ones that do not.
function severityRank(exposure: OrgExposure): number {
  if (exposure.publiclyReachable) return 0;
  if (exposure.state === "live") return 1;
  if (exposure.state === "suspended") return 2;
  return 3;
}

function stateBadgeVariant(state: ExposureState) {
  switch (state) {
    case "live":
      return "default" as const;
    case "suspended":
    case "agent-inactive":
      return "destructive" as const;
    default:
      return "secondary" as const;
  }
}

function relativeTime(value: string | null): string {
  if (!value) return "never";
  const seconds = Math.round((new Date(value).getTime() - Date.now()) / 1000);
  const formatter = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  const abs = Math.abs(seconds);
  if (abs < 60) return formatter.format(seconds, "second");
  if (abs < 3600) return formatter.format(Math.round(seconds / 60), "minute");
  if (abs < 86400) return formatter.format(Math.round(seconds / 3600), "hour");
  return formatter.format(Math.round(seconds / 86400), "day");
}

function daysSince(value: string | null): number | null {
  if (!value) return null;
  return (Date.now() - new Date(value).getTime()) / 86_400_000;
}

type StateFilter = "all" | "live" | "public" | "idle";

/// What in this org is reachable from outside right now.
///
/// An Agent detail page cannot answer this by construction — it shows one
/// agent — which is why this view outlives the Apps list it replaces. Rows
/// link into the owning agent's Integrations tab. The one write here is
/// the agent-level incident action.
export default function ExposuresPage() {
  const { exposures, isLoading } = useOrgExposures();
  const { can } = usePolicies("agents");
  const suspendExposures = useSuspendAgentExposures();
  const resumeExposures = useResumeAgentExposures();
  const [search, setSearch] = useState("");
  const [stateFilter, setStateFilter] = useState<StateFilter>("all");
  const [transport, setTransport] = useState("all");

  usePageTitle("Exposures");

  const canManage = can("agent.manage");

  const transports = useMemo(
    () => Array.from(new Set(exposures.map((exposure) => exposure.channel.channel_type))).sort(),
    [exposures],
  );

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return exposures
      .filter((exposure) => {
        if (transport !== "all" && exposure.channel.channel_type !== transport) return false;
        if (stateFilter === "live" && exposure.state !== "live") return false;
        if (stateFilter === "public" && !exposure.publiclyReachable) return false;
        if (stateFilter === "idle") {
          const days = daysSince(exposure.lastInvokedAt);
          // Never invoked counts as idle: an exposure nobody has ever called is
          // the strongest case of the thing this filter is looking for.
          if (days !== null && days < 30) return false;
        }
        if (!needle) return true;
        const agentName = exposure.agent ? getDisplayName(exposure.agent) : "";
        return (
          agentName.toLowerCase().includes(needle) ||
          exposure.channel.channel_type.toLowerCase().includes(needle) ||
          exposure.channel.id.toLowerCase().includes(needle)
        );
      })
      .sort((a, b) => {
        const rank = severityRank(a) - severityRank(b);
        if (rank !== 0) return rank;
        return new Date(b.lastInvokedAt ?? 0).getTime() - new Date(a.lastInvokedAt ?? 0).getTime();
      });
  }, [exposures, search, stateFilter, transport]);

  const liveCount = exposures.filter((exposure) => exposure.state === "live").length;
  const publicCount = exposures.filter((exposure) => exposure.publiclyReachable).length;
  const suspendedAgents = new Set(
    exposures
      .filter((exposure) => exposure.state === "suspended")
      .map((exposure) => exposure.agent?.id)
      .filter(Boolean),
  ).size;

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Exposures" }]} />

      <PageMasthead
        icon={<Radio />}
        title="Exposures"
        description="Every way into this organization's agents, and every schedule that starts one on its own."
      />

      <PageControlStrip>
        <StatGrid>
          <StatCard
            label="Live"
            value={String(liveCount)}
            hint={`of ${exposures.length} ${pluralize(exposures.length, "exposure")}`}
          />
          <StatCard
            label="Publicly reachable"
            value={String(publicCount)}
            hint="Live and callable with no credential"
          />
          <StatCard
            label="Suspended agents"
            value={String(suspendedAgents)}
            hint="Exposures switched off at the agent"
          />
          <StatCard label="Transports" value={String(transports.length)} hint="Distinct ways in" />
        </StatGrid>
      </PageControlStrip>

      <div className="flex flex-col gap-4">
        <div className="flex flex-wrap items-center gap-3">
          <SearchInput
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search by agent, transport or endpoint id"
            className="min-w-64 flex-1"
          />
          <Select
            value={stateFilter}
            onValueChange={(value) => setStateFilter(value as StateFilter)}
          >
            <SelectTrigger className="w-52" aria-label="Filter by state">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All states</SelectItem>
              <SelectItem value="live">Live only</SelectItem>
              <SelectItem value="public">Publicly reachable</SelectItem>
              <SelectItem value="idle">Idle 30+ days</SelectItem>
            </SelectContent>
          </Select>
          <Select value={transport} onValueChange={setTransport}>
            <SelectTrigger className="w-48" aria-label="Filter by transport">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">All transports</SelectItem>
              {transports.map((kind) => (
                <SelectItem key={kind} value={kind}>
                  {getChannelTypeDisplayName(kind)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>

        {isLoading ? (
          <p className="text-sm text-muted-foreground">Loading exposures…</p>
        ) : filtered.length === 0 ? (
          <EmptyState
            icon={<Radio />}
            title="Nothing matches"
            description={
              exposures.length === 0
                ? "No agent in this organization is exposed yet."
                : "No exposure matches the current filters."
            }
          />
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Exposure</TableHead>
                <TableHead>Agent</TableHead>
                <TableHead>State</TableHead>
                <TableHead>Access</TableHead>
                <TableHead>Last invoked</TableHead>
                <TableHead className="text-right">Incident</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {filtered.map((exposure) => {
                const agentName = exposure.agent ? getDisplayName(exposure.agent) : "unassigned";
                const href = exposure.agent
                  ? `/agents/${exposure.agent.id}?tab=integrations`
                  : undefined;
                return (
                  <TableRow
                    key={exposure.channel.id}
                    // The one row someone is scanning for gets a visual
                    // weight the others do not.
                    className={exposure.publiclyReachable ? "bg-destructive/5" : undefined}
                  >
                    <TableCell>
                      <div className="flex items-center gap-2">
                        <span className="font-medium">
                          {getChannelTypeDisplayName(exposure.channel.channel_type)}
                        </span>
                        {exposure.isTrigger && <Badge variant="outline">trigger</Badge>}
                      </div>
                      <p className="mt-1 font-mono text-xs text-muted-foreground">
                        {exposure.channel.id}
                      </p>
                    </TableCell>
                    <TableCell>
                      {href ? (
                        <Link href={href} className="hover:underline">
                          {agentName}
                        </Link>
                      ) : (
                        <span className="text-muted-foreground">{agentName}</span>
                      )}
                    </TableCell>
                    <TableCell>
                      <Badge variant={stateBadgeVariant(exposure.state)}>
                        {exposureStateLabel(exposure.state)}
                      </Badge>
                    </TableCell>
                    <TableCell>
                      {exposure.anonymous ? (
                        // Anonymous is a property of the configuration, so it
                        // is stated whatever the current state: a suspended
                        // anonymous endpoint is still anonymous, and resuming
                        // its agent opens it. Only one that is *also* live is
                        // an open door, and only that one is styled as one.
                        <span
                          className={
                            exposure.publiclyReachable
                              ? "inline-flex items-center gap-1 text-destructive"
                              : "inline-flex items-center gap-1 text-muted-foreground"
                          }
                        >
                          <Globe className="size-4" />
                          Anonymous{!exposure.publiclyReachable && " (not live)"}
                        </span>
                      ) : (
                        <span className="text-muted-foreground">Authenticated</span>
                      )}
                    </TableCell>
                    <TableCell className="text-muted-foreground">
                      {relativeTime(exposure.lastInvokedAt)}
                    </TableCell>
                    <TableCell className="text-right">
                      {exposure.agent && canManage ? (
                        exposure.state === "suspended" ? (
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() => resumeExposures.mutate(exposure.agent!.id)}
                            disabled={resumeExposures.isPending}
                          >
                            Resume
                          </Button>
                        ) : (
                          <Button
                            size="sm"
                            variant="outline"
                            onClick={() => suspendExposures.mutate(exposure.agent!.id)}
                            disabled={suspendExposures.isPending}
                          >
                            <ShieldAlert className="size-4" />
                            Suspend
                          </Button>
                        )
                      ) : null}
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        )}
      </div>
    </PageContainer>
  );
}
