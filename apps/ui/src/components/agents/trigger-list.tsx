"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import Link from "next/link";
import type { ReactNode } from "react";
import {
  ChevronRight,
  Clock3,
  GitPullRequest,
  MoreHorizontal,
  Play,
  Radio,
  Trash2,
  Webhook,
} from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuPositioner,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { EntityStatus } from "@/components/ui/entity-status";
import { describeCronExpression } from "@/components/apps/cron-label";
import { githubEventsLabel } from "@/components/agents/github-trigger-form";
import { lastUsedLabel } from "@/components/agents/channels/channel-summary";
import { useAgentTriggerRuns } from "@/hooks/use-agent-triggers";
import type { AgentChannel, AgentTrigger } from "@/lib/api/types";

type TriggerConfig = {
  message?: string;
  cron_expression?: string;
  timezone?: string;
  events?: string[];
  repositories?: string[];
  server?: string;
  event?: string;
};

/** Name and one line of facts for a trigger row. Triggers have no name of their own. */
export function triggerSummary(trigger: AgentTrigger): {
  title: string;
  facts: string[];
  icon: ReactNode;
} {
  const config = (trigger.config ?? {}) as TriggerConfig;
  const message = config.message?.trim();
  switch (trigger.trigger_type) {
    case "schedule":
      return {
        title: config.cron_expression ? describeCronExpression(config.cron_expression) : "Schedule",
        facts: ["Schedule", config.timezone?.trim() || "UTC", ...(message ? [message] : [])],
        icon: <Clock3 className="icon-sharp size-4" />,
      };
    case "github":
      return {
        title: githubEventsLabel(config.events ?? []),
        facts: [
          "GitHub",
          config.repositories?.length ? config.repositories.join(", ") : "all repositories",
          ...(config.events?.length ? [config.events.join(", ")] : []),
        ],
        icon: <GitPullRequest className="icon-sharp size-4" />,
      };
    case "mcp_event":
      return {
        title: `${config.event ?? "MCP event"} on ${config.server ?? "MCP server"}`,
        facts: ["MCP event", ...(message ? [message] : [])],
        icon: <Radio className="icon-sharp size-4" />,
      };
    default:
      return {
        title: "Webhook",
        facts: ["Webhook", ...(message ? [message] : [])],
        icon: <Webhook className="icon-sharp size-4" />,
      };
  }
}

const GRID =
  "grid grid-cols-[2rem_minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1 @min-[40rem]/triggers:grid-cols-[2rem_minmax(0,1fr)_7.5rem_8.5rem_4.5rem]";

/// The triggers on the agent's Channels and triggers tab, in the same table
/// shape as the channels above it: one row each, the whole row opens the
/// trigger's page, where its settings, setup and run history live.
export function TriggerList({
  agentId,
  triggers,
  scheduleChannels = [],
  onEnabledChange,
  onRunNow,
  onDelete,
  pending = false,
}: {
  agentId: string;
  triggers: AgentTrigger[];
  /** Schedules created as channels before schedules became triggers. */
  scheduleChannels?: AgentChannel[];
  onEnabledChange?: (trigger: AgentTrigger, enabled: boolean) => void;
  onRunNow?: (trigger: AgentTrigger) => void;
  onDelete?: (trigger: AgentTrigger) => void;
  pending?: boolean;
}) {
  const ui = useUiTranslations();
  return (
    <div className="@container/triggers border bg-card" role="table" aria-label={ui("Triggers")}>
      <div
        role="row"
        className={`hidden border-b bg-muted/30 px-4 py-2 text-xs font-medium text-muted-foreground @min-[40rem]/triggers:grid ${GRID}`}
      >
        <span role="columnheader" className="col-start-2">
          {ui("Trigger")}
        </span>
        <span role="columnheader">{ui("Status")}</span>
        <span role="columnheader">{ui("Last run")}</span>
      </div>
      {triggers.map((trigger) => (
        <TriggerListRow
          key={trigger.id}
          agentId={agentId}
          trigger={trigger}
          onEnabledChange={onEnabledChange}
          onRunNow={onRunNow}
          onDelete={onDelete}
          pending={pending}
        />
      ))}
      {scheduleChannels.map((channel) => {
        const config = (channel.channel_config ?? {}) as TriggerConfig;
        const on = channel.enabled && channel.status === "live";
        return (
          <Row
            key={channel.id}
            href={`/agents/${agentId}/channels/${channel.id}`}
            icon={<Clock3 className="icon-sharp size-4" />}
            title={
              config.cron_expression ? describeCronExpression(config.cron_expression) : "Schedule"
            }
            facts={[
              "Schedule",
              config.timezone?.trim() || "UTC",
              ...(config.message ? [config.message] : []),
            ]}
            on={on}
            lastRun={lastUsedLabel(channel.last_invoked_at)}
          />
        );
      })}
    </div>
  );
}

function TriggerListRow({
  agentId,
  trigger,
  onEnabledChange,
  onRunNow,
  onDelete,
  pending,
}: {
  agentId: string;
  trigger: AgentTrigger;
  onEnabledChange?: (trigger: AgentTrigger, enabled: boolean) => void;
  onRunNow?: (trigger: AgentTrigger) => void;
  onDelete?: (trigger: AgentTrigger) => void;
  pending: boolean;
}) {
  const ui = useUiTranslations();
  const isSchedule = trigger.trigger_type === "schedule";
  const { title, facts, icon } = triggerSummary(trigger);
  // Only schedules keep a run history; event triggers show their deliveries
  // on the trigger's page.
  const { data: runs = [] } = useAgentTriggerRuns(agentId, trigger.id, isSchedule);
  const lastRun = isSchedule ? lastUsedLabel(runs[0]?.scheduled_at) : "—";
  const runNow = isSchedule && onRunNow && trigger.enabled ? onRunNow : undefined;
  const hasMenu = !!runNow || !!onEnabledChange || !!onDelete;
  return (
    <Row
      href={`/agents/${agentId}/triggers/${trigger.id}`}
      icon={icon}
      title={title}
      facts={facts}
      on={trigger.enabled}
      lastRun={lastRun}
      menu={
        hasMenu ? (
          <DropdownMenu>
            <DropdownMenuTrigger
              className={buttonVariants({ variant: "ghost", size: "icon-sm" })}
              aria-label={ui("Actions for {value0}", { value0: title })}
            >
              <MoreHorizontal className="size-4" />
            </DropdownMenuTrigger>
            <DropdownMenuPositioner align="end">
              <DropdownMenuContent>
                {runNow && (
                  <DropdownMenuItem disabled={pending} onClick={() => runNow(trigger)}>
                    <Play className="size-4" />
                    {ui("Run now")}
                  </DropdownMenuItem>
                )}
                {onEnabledChange && (
                  <DropdownMenuItem
                    disabled={pending}
                    onClick={() => onEnabledChange(trigger, !trigger.enabled)}
                  >
                    {trigger.enabled ? ui("Turn off") : ui("Turn on")}
                  </DropdownMenuItem>
                )}
                {onDelete && (runNow || onEnabledChange) && <DropdownMenuSeparator />}
                {onDelete && (
                  <DropdownMenuItem
                    disabled={pending}
                    className="text-destructive"
                    onClick={() => onDelete(trigger)}
                  >
                    <Trash2 className="size-4" />
                    {ui("Delete")}
                  </DropdownMenuItem>
                )}
              </DropdownMenuContent>
            </DropdownMenuPositioner>
          </DropdownMenu>
        ) : null
      }
    />
  );
}

function Row({
  href,
  icon,
  title,
  facts,
  on,
  lastRun,
  menu,
}: {
  href: string;
  icon: ReactNode;
  title: string;
  facts: string[];
  on: boolean;
  lastRun: string;
  menu?: ReactNode;
}) {
  const ui = useUiTranslations();
  const status = (
    <EntityStatus
      status={on ? "on" : "off"}
      tone={on ? "success" : "muted"}
      label={on ? ui("On") : ui("Off")}
    />
  );
  return (
    <div
      role="row"
      className={`relative border-border/60 px-4 py-3.5 text-sm transition-colors hover:bg-muted/50 has-[a:focus-visible]:bg-muted/50 [&+&]:border-t ${GRID}`}
    >
      <span className="flex size-8 items-center justify-center border bg-background" role="cell">
        {icon}
      </span>
      <div className="min-w-0" role="cell">
        {/* The name is the row's link; its overlay makes the whole row clickable. */}
        <Link
          href={href}
          className="block truncate font-semibold outline-none after:absolute after:inset-0"
        >
          {title}
        </Link>
        <p className="mt-0.5 truncate text-muted-foreground">{facts.join(" · ")}</p>
        <div className="mt-1 flex items-center gap-3 @min-[40rem]/triggers:hidden">
          {status}
          <span className="text-xs text-muted-foreground">{lastRun}</span>
        </div>
      </div>
      <span role="cell" className="hidden @min-[40rem]/triggers:block">
        {status}
      </span>
      <span
        role="cell"
        className="hidden text-muted-foreground tabular-nums @min-[40rem]/triggers:block"
      >
        {lastRun}
      </span>
      <span
        role="cell"
        className="relative z-10 flex items-center justify-end gap-1 text-muted-foreground"
      >
        {menu}
        <ChevronRight aria-hidden="true" className="size-4" />
      </span>
    </div>
  );
}
