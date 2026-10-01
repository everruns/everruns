"use client";
import Link from "next/link";
import { useQuery } from "@tanstack/react-query";
import { useOrg } from "@/providers/org-provider";
import { api } from "@/lib/api/client";
interface Session {
  id: string;
  title?: string | null;
  status: string;
  updated_at: string;
}
export function VirtualUserSessions({ identityId }: { identityId: string }) {
  const { currentOrg } = useOrg();
  const query = useQuery({
    queryKey: ["virtual-user-sessions", identityId, currentOrg?.public_id],
    enabled: !!currentOrg,
    queryFn: async () =>
      (await api.get<{ data: Session[] }>(`/v1/virtual-users/${identityId}/sessions`)).data.data,
  });
  if (query.isLoading) return <p>Loading sessions…</p>;
  if (query.error) return <p className="text-destructive">Sessions could not be loaded.</p>;
  return (
    <div className="space-y-2">
      {!query.data?.length && <p className="text-sm text-muted-foreground">No sessions yet.</p>}
      {query.data?.map((session) => (
        <Link
          key={session.id}
          href={`/sessions/${session.id}`}
          className="flex justify-between gap-3 border p-3 text-sm hover:bg-muted"
        >
          <span>{session.title || session.id}</span>
          <span className="text-muted-foreground">{session.status}</span>
        </Link>
      ))}
    </div>
  );
}
