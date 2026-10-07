"use client";
import Link from "next/link";
import { useUpdateAgent } from "@/hooks";
import { VirtualUserSelect } from "@/components/virtual-user/virtual-user-select";

/** Binding editor. The agent page opens this from a More row; it saves on its own. */
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
    <div className="space-y-3">
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
    </div>
  );
}
