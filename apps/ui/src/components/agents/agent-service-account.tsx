"use client";
import Link from "next/link";
import { useUpdateAgent } from "@/hooks";
import { VirtualUserSelect } from "@/components/virtual-user/virtual-user-select";
import { Card, CardHeader, CardTitle, CardContent, CardDescription } from "@/components/ui/card";
export function AgentServiceAccount({
  agentId,
  value,
  disabled,
}: {
  agentId: string;
  value?: string | null;
  disabled?: boolean;
}) {
  const update = useUpdateAgent();
  return (
    <Card>
      <CardHeader>
        <CardTitle>Service account</CardTitle>
        <CardDescription>
          Connections the agent uses for operations configured to act as a service.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-3">
        <VirtualUserSelect
          value={value ?? ""}
          usage="service"
          disabled={disabled || update.isPending}
          noneLabel="Create a service account on first use"
          onValueChange={(id) =>
            update.mutate({ agentId, request: { service_virtual_user_id: id || null } })
          }
        />
        {value && (
          <Link className="text-sm text-primary underline" href={`/virtual-users/${value}`}>
            Manage service account and connections
          </Link>
        )}
        {update.error && <p className="text-sm text-destructive">{update.error.message}</p>}
      </CardContent>
    </Card>
  );
}
