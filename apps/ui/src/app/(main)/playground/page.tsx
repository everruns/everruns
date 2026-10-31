"use client";
import { SegmentedControl } from "@/components/ui/segmented-control";

import { useLocale as useDisplayLocale, useUiTranslations } from "@/providers/locale-provider";

import { useChatPageFilters } from "@/components/chat-mode/page-view-state";

import { PlaygroundIcon } from "@/components/icons/facet-icons";
import { AgentIcon } from "@/components/icons/facet-icons";
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ListFilter, Plus, Users, X } from "lucide-react";
import { getSessionFacets, listSessions } from "@/lib/api/sessions";
import { getDisplayName } from "@/lib/entity-lifecycle";
import type { PlaygroundGroupBy } from "@/lib/playground-list";
import { useOrg } from "@/providers/org-provider";
import { useAgents, usePageTitle } from "@/hooks";
import { Button, LinkButton, buttonVariants } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPositioner,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { PlaygroundChatList } from "@/components/playground/playground-chat-list";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import {
  PageBreadcrumb,
  PageContainer,
  PageControlStrip,
  PageFooter,
  PageMasthead,
  SectionTabs,
} from "@/components/layout/page-layout";
import { cn } from "@/lib/utils";

const PAGE_SIZE = 20;

const GROUP_OPTIONS: Array<{ value: PlaygroundGroupBy; label: string }> = [
  { value: "day", label: "Day" },
  { value: "agent", label: "Agent" },
  { value: "none", label: "None" },
];

function PlaygroundLibrary() {
  const { displayLocale } = useDisplayLocale();
  const ui = useUiTranslations();
  const { currentOrg } = useOrg();
  const { data: agents = [] } = useAgents();
  const [search, setSearch] = useState("");
  useChatPageFilters({ search: search });
  const [agentId, setAgentId] = useState<string | null>(null);
  const [archived, setArchived] = useState(false);
  const [groupBy, setGroupBy] = useState<PlaygroundGroupBy>("day");
  const [page, setPage] = useState(0);
  const orgId = currentOrg?.public_id;
  const { data, isLoading, error } = useQuery({
    queryKey: ["sessions", "playground", orgId, search, agentId, archived, page],
    queryFn: () =>
      listSessions({
        source: "playground",
        search,
        agentId: agentId ?? undefined,
        archivedOnly: archived,
        order: "last_activity",
        limit: PAGE_SIZE,
        offset: page * PAGE_SIZE,
      }),
    enabled: !!currentOrg,
  });
  const { data: activeFacets } = useQuery({
    queryKey: ["sessions", "playground", "facets", "active", orgId],
    queryFn: () => getSessionFacets({ source: "playground" }),
    enabled: !!currentOrg,
  });
  const { data: agentFacets, isLoading: agentFacetsLoading } = useQuery({
    queryKey: ["sessions", "playground", "facets", "agents", orgId, search, archived],
    queryFn: () =>
      getSessionFacets({
        source: "playground",
        search: search || undefined,
        archivedOnly: archived,
      }),
    enabled: !!currentOrg,
  });
  usePageTitle(ui("Playground"));

  const selectedAgent = agents.find((agent) => agent.id === agentId);
  const selectedAgentLabel = agentId
    ? selectedAgent
      ? getDisplayName(selectedAgent, displayLocale)
      : agentId
    : null;
  const filtering = Boolean(search || agentId);
  const agentOptions = (agentFacets?.by_agent ?? []).map((bucket) => {
    const agent = agents.find((candidate) => candidate.id === bucket.value);
    return {
      id: bucket.value,
      label: agent ? getDisplayName(agent, displayLocale) : bucket.value,
      count: bucket.count,
    };
  });

  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: ui("Playground") }]} />
      <PageMasthead
        icon={<PlaygroundIcon />}
        title={ui("Playground")}
        badges={activeFacets ? <Badge variant="outline">{activeFacets.total}</Badge> : undefined}
        description={ui(
          "Test agents as virtual users. Playground chats are shared with your organisation.",
        )}
        actions={
          <LinkButton href="/playground/new" variant="accent">
            <Plus className="size-4" />
            {ui("New chat")}
          </LinkButton>
        }
      />
      <PageControlStrip className="flex flex-wrap items-center gap-2">
        <SearchInput
          containerClassName="w-64"
          aria-label={ui("Search Playground chats")}
          placeholder={ui("Search Playground chats…")}
          value={search}
          onChange={(event) => {
            setSearch(event.target.value);
            setPage(0);
          }}
        />
        <DropdownMenu>
          <DropdownMenuTrigger
            className={cn(
              buttonVariants({ variant: "outline" }),
              "border-dashed bg-transparent text-muted-foreground shadow-none hover:bg-muted",
            )}
          >
            <ListFilter className="size-3.5 opacity-70" />
            {ui("Filter")}
          </DropdownMenuTrigger>
          <DropdownMenuPositioner align="start">
            <DropdownMenuContent className="w-56">
              <DropdownMenuGroup>
                <DropdownMenuLabel>{ui("Agent")}</DropdownMenuLabel>
                {agentOptions.length === 0 ? (
                  <DropdownMenuItem disabled>
                    {agentFacetsLoading ? ui("Loading agents") : ui("No agents")}
                  </DropdownMenuItem>
                ) : (
                  agentOptions.map((option) => (
                    <DropdownMenuItem
                      key={option.id}
                      onClick={() => {
                        setAgentId(option.id);
                        setPage(0);
                      }}
                    >
                      <AgentIcon className="size-3.5 opacity-60" />
                      <span className="min-w-0 flex-1 truncate">{option.label}</span>
                      <span className="font-mono text-[11px] text-muted-foreground">
                        {option.count}
                      </span>
                    </DropdownMenuItem>
                  ))
                )}
              </DropdownMenuGroup>
            </DropdownMenuContent>
          </DropdownMenuPositioner>
        </DropdownMenu>
        {selectedAgentLabel && (
          <span className="inline-flex h-7 items-center gap-1.5 border bg-card pr-1 pl-2 text-xs">
            <span className="text-muted-foreground">{ui("Agent is")}</span>
            <span className="font-medium">{selectedAgentLabel}</span>
            <button
              type="button"
              aria-label={ui("Remove filter")}
              className="inline-flex size-5 items-center justify-center hover:bg-muted"
              onClick={() => {
                setAgentId(null);
                setPage(0);
              }}
            >
              <X className="size-3 opacity-60" />
            </button>
          </span>
        )}
        <div className="flex-1" />
        <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <span id="playground-group-label">{ui("Group by")}</span>
          <SegmentedControl
            label={ui("Group by")}
            value={groupBy}
            onValueChange={(value) => setGroupBy(value as PlaygroundGroupBy)}
            options={GROUP_OPTIONS}
          />
        </div>
        <SectionTabs
          className="ml-2 w-auto shrink-0 gap-3 border-b-0"
          value={archived ? "archived" : "active"}
          onValueChange={(value) => {
            setArchived(value === "archived");
            setPage(0);
          }}
          items={[
            { value: "active", label: ui("Active") },
            { value: "archived", label: ui("Archived") },
          ]}
        />
      </PageControlStrip>
      {error ? (
        <ChatErrorAlert message="Could not load Playground chats." />
      ) : isLoading ? (
        <Skeleton className="h-56 w-full" />
      ) : data?.data.length ? (
        <PlaygroundChatList sessions={data.data} agents={agents} groupBy={groupBy} />
      ) : (
        <div className="flex flex-col items-center gap-2 border bg-card px-6 py-12 text-center">
          <PlaygroundIcon className="size-5 text-muted-foreground" />
          <div className="text-sm font-semibold">
            {filtering
              ? ui("No matching Playground chats")
              : archived
                ? ui("No archived Playground chats")
                : ui("No Playground chats yet")}
          </div>
          <p className="text-[13px] text-muted-foreground">
            {ui("Choose an agent and a virtual user to start a shared chat.")}
          </p>
        </div>
      )}
      <PageFooter>
        <span className="flex items-center gap-2">
          <Users className="size-3.5" />
          {ui("Visible to everyone in")} {currentOrg?.name ?? ui("your organisation")}
        </span>
        {!!data?.total && (
          <div className="flex items-center gap-4">
            <span>
              {ui("Showing")} {page * PAGE_SIZE + 1}–{Math.min((page + 1) * PAGE_SIZE, data.total)}{" "}
              {ui("of")} {data.total}
            </span>
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={page === 0}
                onClick={() => setPage(page - 1)}
              >
                {ui("Previous")}
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={(page + 1) * PAGE_SIZE >= data.total}
                onClick={() => setPage(page + 1)}
              >
                {ui("Next")}
              </Button>
            </div>
          </div>
        )}
      </PageFooter>
    </PageContainer>
  );
}

export default function PlaygroundPage() {
  const { currentOrg } = useOrg();
  return <PlaygroundLibrary key={currentOrg?.public_id} />;
}
