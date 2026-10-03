"use client";

import { useState } from "react";
import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { FlaskConical, Plus, Users } from "lucide-react";
import { listSessions } from "@/lib/api/sessions";
import { threadTitle } from "@/lib/chat-threads";
import { activityLabel } from "@/lib/session-filters";
import { useOrg } from "@/providers/org-provider";
import { useAgents, usePageTitle } from "@/hooks";
import { useVirtualUser } from "@/hooks/use-virtual-users";
import { getDisplayName } from "@/lib/entity-lifecycle";
import { Button, LinkButton } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
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

function SubjectName({ id }: { id?: string | null }) {
  const { data } = useVirtualUser(id ?? undefined);
  return <>{data?.name ?? id ?? "—"}</>;
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
    <div className="mx-auto w-full max-w-7xl space-y-6 p-4 sm:p-8">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div>
          <h1 className="flex items-center gap-2 text-2xl font-semibold tracking-tight">
            <FlaskConical className="size-6 text-primary" />
            Playground
          </h1>
          <p className="mt-2 text-sm text-muted-foreground">
            Try agents as virtual users. Every conversation is shared with your organisation.
          </p>
        </div>
        <LinkButton href="/playground/new">
          <Plus className="size-4" />
          New conversation
        </LinkButton>
      </header>
      <div className="flex flex-wrap items-center gap-3 border-y py-3">
        <div className="flex gap-1">
          <Button
            variant={!archived ? "secondary" : "ghost"}
            size="sm"
            onClick={() => {
              setArchived(false);
              setPage(0);
            }}
          >
            Active
          </Button>
          <Button
            variant={archived ? "secondary" : "ghost"}
            size="sm"
            onClick={() => {
              setArchived(true);
              setPage(0);
            }}
          >
            Archived
          </Button>
        </div>
        <Input
          className="min-w-48 flex-1"
          aria-label="Search conversations"
          placeholder="Search conversations…"
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
      </div>
      {error ? (
        <ChatErrorAlert message="Could not load Playground conversations." />
      ) : isLoading ? (
        <Skeleton className="h-56 w-full" />
      ) : data?.data.length ? (
        <div className="border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Conversation</TableHead>
                <TableHead>Agent</TableHead>
                <TableHead>Talk as</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Last activity</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.data.map((session) => (
                <TableRow key={session.id}>
                  <TableCell className="max-w-sm">
                    <Link
                      href={`/playground/${session.id}`}
                      className="block truncate font-medium hover:text-primary"
                    >
                      {threadTitle(session, "New conversation")}
                    </Link>
                    <p className="mt-1 truncate text-xs text-muted-foreground">
                      {session.preview ?? "No messages yet"}
                    </p>
                  </TableCell>
                  <TableCell>
                    {(() => {
                      const agent = agents.find((a) => a.id === session.agent_id);
                      return agent ? getDisplayName(agent) : "Harness conversation";
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
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      ) : (
        <div className="border border-dashed px-6 py-16 text-center">
          <FlaskConical className="mx-auto mb-3 size-8 text-muted-foreground" />
          <h2 className="font-medium">
            {archived
              ? "No archived conversations"
              : search || agent !== "all" || subject
                ? "No matching conversations"
                : "Start your first experiment"}
          </h2>
          <p className="mt-2 text-sm text-muted-foreground">
            Choose an agent and a virtual user to try a conversation together.
          </p>
        </div>
      )}
      {!!data?.total && (
        <div className="flex items-center justify-between text-sm text-muted-foreground">
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
      <p className="flex items-center gap-2 text-xs text-muted-foreground">
        <Users className="size-3.5" />
        Visible to everyone in {currentOrg?.name ?? "your organisation"}
      </p>
    </div>
  );
}

export default function PlaygroundPage() {
  const { currentOrg } = useOrg();
  return <PlaygroundLibrary key={currentOrg?.public_id} />;
}
