"use client";

import { useMemo, useState } from "react";
import { useApps, usePublishApp, useUnpublishApp } from "@/hooks/use-apps";
import { usePageTitle } from "@/hooks";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { SearchInput } from "@/components/ui/search-input";
import { EntityCard, EntityCardFooter } from "@/components/ui/entity-card";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { Plus, Rocket, Globe, GlobeLock, Copy, Clock3 } from "lucide-react";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageControlStrip,
  SectionTabs,
  EmptyState,
  PageColumns,
  PageMain,
  PageRail,
  RailSection,
  PageFooter,
  IconTile,
} from "@/components/layout";
import Link from "next/link";
import type { App, AppChannel, ChannelType, ScheduleChannelConfig } from "@/lib/api/types";
import {
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isArchivedStatus,
} from "@/lib/entity-lifecycle";
import { getChannelTypeDisplayName } from "@/lib/app-channels";
import { pluralize } from "@/lib/formatting";

type StatusTab = "all" | "active" | "archived";

export default function AppsPage() {
  usePageTitle("Apps");
  const [statusTab, setStatusTab] = useState<StatusTab>("active");
  const [search, setSearch] = useState("");
  // Fetch the full set (including archived) so the facet rail can show accurate counts.
  const { data: apps, isLoading, error } = useApps({ includeArchived: true });

  const counts = useMemo(() => {
    const list = apps ?? [];
    const archived = list.filter((a) => isArchivedStatus(a.status)).length;
    return { all: list.length, active: list.length - archived, archived };
  }, [apps]);

  const channelFacets = useMemo(() => {
    const tally = new Map<ChannelType, number>();
    for (const app of apps ?? []) {
      for (const channel of app.channels) {
        tally.set(channel.channel_type, (tally.get(channel.channel_type) ?? 0) + 1);
      }
    }
    return [...tally.entries()]
      .map(([type, count]) => ({ type, count, name: getChannelTypeDisplayName(type) }))
      .sort((a, b) => b.count - a.count);
  }, [apps]);

  const filteredApps = useMemo(() => {
    const query = search.trim().toLowerCase();
    return (apps ?? []).filter((app) => {
      if (statusTab === "active" && isArchivedStatus(app.status)) return false;
      if (statusTab === "archived" && !isArchivedStatus(app.status)) return false;
      if (!query) return true;
      const haystack = [app.name, app.description].filter(Boolean).join(" ").toLowerCase();
      return haystack.includes(query);
    });
  }, [apps, search, statusTab]);

  const statusItems = [
    { value: "all" as const, label: "All" },
    { value: "active" as const, label: "Active" },
    { value: "archived" as const, label: "Archived" },
  ];

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Apps" }]} />

      <PageMasthead
        icon={<Rocket />}
        title="Apps"
        badges={
          <Badge variant="outline" className="font-mono">
            {counts.all}
          </Badge>
        }
        description="Deploy agents to channels — Slack, AG-UI, webhooks, and more."
        meta={
          <>
            <span>{counts.active} active</span>
            <span>{counts.archived} archived</span>
          </>
        }
        actions={
          <Link href="/apps/new">
            <Button variant="accent">
              <Plus className="size-4" />
              New app
            </Button>
          </Link>
        }
      />

      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <SearchInput
          placeholder="Search apps…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          containerClassName="w-64"
        />
        <div className="flex-1" />
        <SectionTabs
          value={statusTab}
          onValueChange={(v) => setStatusTab(v as StatusTab)}
          items={statusItems}
        />
      </PageControlStrip>

      <PageColumns>
        <PageMain>
          <QueryStateWrapper
            isLoading={isLoading}
            error={error}
            data={filteredApps}
            errorMessagePrefix="Failed to load apps"
            skeletonCount={3}
            emptyState={
              <EmptyState
                icon={<Rocket />}
                title={
                  search || statusTab !== "active" ? "No apps match your filters." : "No apps yet"
                }
                description={
                  !search && statusTab === "active"
                    ? "Apps deploy your agents to channels like Slack and AG-UI. Create an app to connect an agent to the interface you need, or invoke it through authenticated webhooks. Configure schedules from the agent's Triggers tab."
                    : undefined
                }
                action={
                  !search &&
                  statusTab === "active" && (
                    <Link href="/apps/new">
                      <Button variant="accent">
                        <Plus className="size-4" />
                        Create your first app
                      </Button>
                    </Link>
                  )
                }
              />
            }
          >
            {(items) => (
              <div className="grid gap-4 md:grid-cols-2">
                {items.map((app) => (
                  <AppCard key={app.id} app={app} />
                ))}
              </div>
            )}
          </QueryStateWrapper>
        </PageMain>

        <PageRail>
          <RailSection label="Status">
            <div className="flex flex-col gap-1.5 text-[13px]">
              {statusItems.map((item) => (
                <button
                  key={item.value}
                  type="button"
                  onClick={() => setStatusTab(item.value)}
                  className={
                    statusTab === item.value
                      ? "flex items-center justify-between text-foreground transition-colors hover:text-foreground"
                      : "flex items-center justify-between text-muted-foreground transition-colors hover:text-foreground"
                  }
                >
                  <span>{item.label}</span>
                  <span className="text-muted-foreground">{counts[item.value]}</span>
                </button>
              ))}
            </div>
          </RailSection>

          {channelFacets.length > 0 && (
            <RailSection label="Channels">
              <div className="flex flex-col gap-1.5 text-[13px] text-muted-foreground">
                {channelFacets.map((facet) => (
                  <div key={facet.type} className="flex items-center justify-between">
                    <span>{facet.name}</span>
                    <span>{facet.count}</span>
                  </div>
                ))}
              </div>
            </RailSection>
          )}
        </PageRail>
      </PageColumns>

      <PageFooter>
        <span>
          Showing {filteredApps.length} of {counts.all} {pluralize(counts.all, "app")}
        </span>
        {counts.archived > 0 && statusTab !== "archived" && (
          <button
            type="button"
            onClick={() => setStatusTab("archived")}
            className="text-primary transition-colors hover:underline"
          >
            View archived →
          </button>
        )}
      </PageFooter>
    </PageContainer>
  );
}

function AppCard({ app }: { app: App }) {
  const publishApp = usePublishApp();
  const unpublishApp = useUnpublishApp();
  const isPublished = app.status === "published";
  const isArchived = app.status === "archived";
  const primaryChannel = app.channels.find((channel) => channel.enabled) ?? app.channels[0];
  const primaryAccess = getPrimaryChannelAccess(app.id, primaryChannel);

  return (
    <EntityCard
      icon={<IconTile size="md" icon={<Rocket />} />}
      title={app.name}
      href={`/apps/${app.id}`}
      titleClassName={getEntityNameClassName(app.status)}
      copyValue={app.id}
      headerActions={<Badge variant={getEntityStatusBadgeVariant(app.status)}>{app.status}</Badge>}
      footer={
        <EntityCardFooter
          meta={
            <>
              {app.channels.length === 0
                ? "No channels"
                : `${app.channels.length} ${pluralize(app.channels.length, "channel")}`}
            </>
          }
          actions={
            // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions, jsx-a11y/no-noninteractive-element-interactions -- group container only suppresses click propagation; nested actions own interactivity
            <div
              role="group"
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
              }}
            >
              {isPublished ? (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => unpublishApp.mutate(app.id)}
                  disabled={unpublishApp.isPending || isArchived}
                >
                  <GlobeLock className="mr-1 size-3" />
                  Unpublish
                </Button>
              ) : (
                <Button
                  variant="default"
                  size="sm"
                  onClick={() => publishApp.mutate(app.id)}
                  disabled={publishApp.isPending || isArchived}
                >
                  <Globe className="mr-1 size-3" />
                  Publish
                </Button>
              )}
            </div>
          }
        />
      }
    >
      {app.description ? (
        <p className="mb-3 line-clamp-2 text-sm text-muted-foreground">{app.description}</p>
      ) : (
        <p className="mb-3 text-sm italic text-muted-foreground">No description provided</p>
      )}

      {app.channels.length > 0 && (
        <div className="mb-3 flex flex-wrap items-center gap-1 text-xs text-muted-foreground">
          {app.channels.map((channel) => (
            <Badge key={channel.id} variant="outline" className="text-xs">
              {getChannelTypeDisplayName(channel.channel_type)}
            </Badge>
          ))}
        </div>
      )}

      {isPublished && primaryAccess && (
        <div className="mb-3 flex items-center gap-1 rounded bg-muted p-2 text-xs text-muted-foreground">
          {primaryAccess.kind === "schedule" ? (
            <Clock3 className="size-3 shrink-0" />
          ) : (
            <Globe className="size-3 shrink-0" />
          )}
          <span className="truncate font-mono">{primaryAccess.value}</span>
          {primaryAccess.copyable && (
            <button
              className="ml-1 shrink-0 hover:text-foreground"
              onClick={(e) => {
                e.preventDefault();
                e.stopPropagation();
                navigator.clipboard.writeText(primaryAccess.value);
              }}
            >
              <Copy className="size-3" />
            </button>
          )}
        </div>
      )}
    </EntityCard>
  );
}

function getPrimaryChannelAccess(appId: string, channel: AppChannel | undefined) {
  if (!channel) {
    return null;
  }

  const origin = typeof window !== "undefined" ? window.location.origin : "";

  switch (channel.channel_type) {
    case "ag_ui":
      return {
        kind: "url" as const,
        value: origin ? `${origin}/api/v1/apps/${appId}/ag-ui` : `/api/v1/apps/${appId}/ag-ui`,
        copyable: true,
      };
    case "slack":
      return {
        kind: "url" as const,
        value: origin
          ? `${origin}/api/v1/apps/${appId}/slack/events`
          : `/api/v1/apps/${appId}/slack/events`,
        copyable: true,
      };
    case "webhook":
      return {
        kind: "url" as const,
        value: origin
          ? `${origin}/api/v1/apps/${appId}/webhooks/${channel.id}`
          : `/api/v1/apps/${appId}/webhooks/${channel.id}`,
        copyable: true,
      };
    case "schedule": {
      const config = channel.channel_config as ScheduleChannelConfig;
      return {
        kind: "schedule" as const,
        value: `${config.cron_expression} (${config.timezone ?? "UTC"})`,
        copyable: false,
      };
    }
    case "public_chat":
      return {
        kind: "url" as const,
        value: origin ? `${origin}/public-chat/${appId}` : `/public-chat/${appId}`,
        copyable: true,
      };
  }
}
