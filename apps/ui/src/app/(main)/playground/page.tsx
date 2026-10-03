"use client";

import { useState } from "react";
import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { ArrowRight, FlaskConical, Plus, Users } from "lucide-react";
import { listSessions } from "@/lib/api/sessions";
import { threadTitle } from "@/lib/chat-threads";
import { activityLabel } from "@/lib/session-filters";
import { useOrg } from "@/providers/org-provider";
import { useAgents, usePageTitle } from "@/hooks";
import { useVirtualUser } from "@/hooks/use-virtual-users";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { Button, LinkButton } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
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
import { VirtualUserSelect } from "@/components/virtual-user/virtual-user-select";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import {
  EmptyState,
  PageBreadcrumb,
  PageContainer,
  PageControlStrip,
  PageFooter,
  PageMasthead,
  SectionTabs,
} from "@/components/layout/page-layout";

function SubjectName({ id }: { id?: string | null }) {
  const { data } = useVirtualUser(id ?? undefined);
  if (!id) return <>—</>;
  return (
    <Link href={`/virtual-users/${id}`} className="underline underline-offset-4 hover:text-primary">
      {data?.name ?? id}
    </Link>
  );
}

function PlaygroundLibrary() {
  const { currentOrg } = useOrg();
  const { data: agents = [] } = useAgents();
  const [search, setSearch] = useState("");
  const [agent, setAgent] = useState("all");
  const [subject, setSubject] = useState("");
  const [archived, setArchived] = useState(false);
  const [page, setPage] = useState(0);
  const { data, isLoading, error } = useQuery({
    queryKey: [
      "sessions",
      "playground",
      currentOrg?.public_id,
      search,
      agent,
      subject,
      archived,
      page,
    ],
    queryFn: () =>
      listSessions({
        source: "playground",
        search,
        agentId: agent === "all" ? undefined : agent,
        playgroundUserId: subject || undefined,
        archivedOnly: archived,
        order: "last_activity",
        limit: 20,
        offset: page * 20,
      }),
    enabled: !!currentOrg,
  });
  usePageTitle("Playground");
  return (
    <PageContainer>
      <PageBreadcrumb items={[{ label: "Playground" }]} />
      <PageMasthead
        icon={<FlaskConical />}
        title="Playground"
        badges={data && <Badge variant="outline">{data.total}</Badge>}
        description="Test agents as virtual users. Playground chats are shared with your organisation."
        actions={
          <LinkButton href="/playground/new" variant="accent">
            <Plus className="size-4" />
            New Playground chat
          </LinkButton>
        }
      />
      <PageControlStrip className="flex flex-wrap items-center gap-3">
        <SearchInput
          containerClassName="w-64"
          aria-label="Search Playground chats"
          placeholder="Search Playground chats…"
          value={search}
          onChange={(e) => {
            setSearch(e.target.value);
            setPage(0);
          }}
        />
        <Select
          value={agent}
          onValueChange={(value) => {
            setAgent(value);
            setPage(0);
          }}
        >
          <SelectTrigger className="w-48" aria-label="Filter by agent">
            <SelectValue placeholder="All agents" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All agents</SelectItem>
            {agents.map((a) => (
              <SelectItem key={a.id} value={a.id}>
                {getDisplayName(a)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <VirtualUserSelect
          usage="end_user"
          value={subject}
          onValueChange={(value) => {
            setSubject(value);
            setPage(0);
          }}
          noneLabel="All virtual users"
          placeholder="Filter by virtual user"
          className="w-52"
        />
        <div className="flex-1" />
        <SectionTabs
          value={archived ? "archived" : "active"}
          onValueChange={(value) => {
            setArchived(value === "archived");
            setPage(0);
          }}
          items={[
            { value: "active", label: "Active" },
            { value: "archived", label: "Archived" },
          ]}
        />
      </PageControlStrip>
      {error ? (
        <ChatErrorAlert message="Could not load Playground chats." />
      ) : isLoading ? (
        <Skeleton className="h-56 w-full" />
      ) : data?.data.length ? (
        <div className="border bg-card">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Playground chat</TableHead>
                <TableHead>Agent</TableHead>
                <TableHead>Talk as</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Last activity</TableHead>
                <TableHead>
                  <span className="sr-only">Actions</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.data.map((session) => (
                <TableRow key={session.id}>
                  <TableCell className="max-w-sm">
                    <Link
                      href={`/playground/${session.id}`}
                      className="block truncate font-medium underline underline-offset-4 hover:text-primary"
                    >
                      {threadTitle(session, "New Playground chat")}
                    </Link>
                    <p className="mt-1 truncate text-xs text-muted-foreground">
                      {session.preview ?? "No messages yet"}
                    </p>
                  </TableCell>
                  <TableCell>
                    {(() => {
                      const agent = agents.find((a) => a.id === session.agent_id);
                      return agent ? (
                        <Link
                          href={`/agents/${agent.id}`}
                          className="underline underline-offset-4 hover:text-primary"
                        >
                          {getDisplayName(agent)}
                        </Link>
                      ) : (
                        "Harness chat"
                      );
                    })()}
                  </TableCell>
                  <TableCell>
                    <SubjectName id={session.playground_user_id} />
                  </TableCell>
                  <TableCell>
                    <Badge variant="outline">{activityLabel(session.activity ?? "idle")}</Badge>
                  </TableCell>
                  <TableCell className="whitespace-nowrap text-muted-foreground">
                    {new Date(session.updated_at).toLocaleString()}
                  </TableCell>
                  <TableCell className="text-right">
                    <LinkButton
                      href={`/playground/${session.id}`}
                      variant="outline"
                      size="sm"
                      aria-label={`Open chat ${threadTitle(session, "New Playground chat")}`}
                    >
                      Open chat <ArrowRight className="size-3.5" />
                    </LinkButton>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : (
        <EmptyState
          icon={<FlaskConical />}
          title={
            archived
              ? "No archived Playground chats"
              : search || agent !== "all" || subject
                ? "No matching Playground chats"
                : "No Playground chats yet"
          }
          description="Choose an agent and a virtual user to start a shared chat."
        />
      )}
      <PageFooter>
        <span className="flex items-center gap-2">
          <Users className="size-3.5" />
          Visible to everyone in {currentOrg?.name ?? "your organisation"}
        </span>
        {!!data?.total && (
          <div className="flex items-center gap-4">
            <span>
              {page * 20 + 1}–{Math.min((page + 1) * 20, data.total)} of {data.total}
            </span>
            <div className="flex gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={page === 0}
                onClick={() => setPage(page - 1)}
              >
                Previous
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={(page + 1) * 20 >= data.total}
                onClick={() => setPage(page + 1)}
              >
                Next
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
