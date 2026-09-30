"use client";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useOrg } from "@/providers/org-provider";
import { api } from "@/lib/api/client";
import { Button } from "@/components/ui/button";
interface Binding {
  id: string;
  provider: string;
  realm: string;
  subject: string;
  management_user_id?: string | null;
}
export function LinkedIdentities({ identityId }: { identityId: string }) {
  const { currentOrg } = useOrg();
  const client = useQueryClient();
  const key = ["virtual-user-bindings", identityId, currentOrg?.public_id];
  const query = useQuery({
    queryKey: key,
    enabled: !!currentOrg,
    queryFn: async () =>
      (await api.get<{ data: Binding[] }>(`/v1/virtual-users/${identityId}/bindings`)).data.data,
  });
  const unlink = useMutation({
    mutationFn: async (binding: string) =>
      api.delete(`/v1/virtual-users/${identityId}/bindings/${binding}`),
    onSuccess: () => client.invalidateQueries({ queryKey: key }),
  });
  if (query.isLoading) return <p>Loading linked identities…</p>;
  if (query.error)
    return <p className="text-destructive">Linked identities could not be loaded.</p>;
  return (
    <div className="space-y-3">
      {!query.data?.length && (
        <p className="text-sm text-muted-foreground">No linked identities.</p>
      )}
      {query.data?.map((binding) => (
        <div key={binding.id} className="flex items-start justify-between gap-3 border p-3">
          <div className="min-w-0">
            <p className="text-sm font-medium">{binding.provider}</p>
            <p className="break-all text-xs text-muted-foreground">
              {binding.realm} · {binding.subject}
            </p>
          </div>
          {!binding.management_user_id && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={unlink.isPending}
              onClick={() => {
                if (
                  confirm("Unlink this identity? Its current runtime credentials will be revoked.")
                )
                  unlink.mutate(binding.id);
              }}
            >
              Unlink
            </Button>
          )}
        </div>
      ))}
      {unlink.error && <p className="text-sm text-destructive">Identity could not be unlinked.</p>}
    </div>
  );
}
