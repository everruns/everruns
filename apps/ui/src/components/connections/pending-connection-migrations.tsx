"use client";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api/client";
import { useOrg } from "@/providers/org-provider";
import { Button } from "@/components/ui/button";
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@/components/ui/card";
interface PendingConnection {
  id: string;
  provider: string;
  provider_username?: string;
}
export function PendingConnectionMigrations() {
  const { currentOrg } = useOrg();
  const client = useQueryClient();
  const pending = useQuery({
    queryKey: ["pending-connection-migrations"],
    queryFn: async () =>
      (await api.get<{ data: PendingConnection[] }>("/v1/user/connection-migrations")).data.data,
  });
  const move = useMutation({
    mutationFn: async (id: string) => api.post(`/v1/user/connection-migrations/${id}`),
    onSuccess: () => {
      client.invalidateQueries({ queryKey: ["pending-connection-migrations"] });
      client.invalidateQueries({ queryKey: ["identity-connections"] });
      client.invalidateQueries({ queryKey: ["user-connections"] });
    },
  });
  if (!pending.data?.length) return null;
  return (
    <Card>
      <CardHeader>
        <CardTitle>Choose where your connections belong</CardTitle>
        <CardDescription>
          Your previous connections need an organization. Switch organizations above to choose
          another destination.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        {pending.data.map((connection) => (
          <div key={connection.id} className="flex items-center justify-between gap-3">
            <span>{connection.provider_username || connection.provider}</span>
            <Button
              disabled={!currentOrg || move.isPending}
              onClick={() => move.mutate(connection.id)}
            >
              Move to {currentOrg?.name}
            </Button>
          </div>
        ))}
        {move.error && (
          <p className="text-sm text-destructive">
            This connection could not be moved. The destination may already have this provider.
          </p>
        )}
      </CardContent>
    </Card>
  );
}
