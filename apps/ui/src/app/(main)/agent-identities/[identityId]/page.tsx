"use client";

import Link from "next/link";
import { use } from "react";
import { useRouter } from "next/navigation";
import { ArrowLeft, Trash2 } from "lucide-react";
import {
  useAgentIdentity,
  useDeleteAgentIdentity,
  useDestroyAgentIdentity,
  useUpdateAgentIdentity,
} from "@/hooks/use-agent-identities";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { CopyButton } from "@/components/ui/copy-button";
import { getEntityStatusBadgeVariant } from "@/lib/entity-lifecycle";

export default function AgentIdentityDetailPage({
  params,
}: {
  params: Promise<{ identityId: string }>;
}) {
  const { identityId } = use(params);
  const router = useRouter();
  const { data: identity, isLoading } = useAgentIdentity(identityId);
  const updateIdentity = useUpdateAgentIdentity();
  const deleteIdentity = useDeleteAgentIdentity();
  const destroyIdentity = useDestroyAgentIdentity();

  if (isLoading)
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-64 w-full" />
      </div>
    );
  if (!identity)
    return <div className="container mx-auto p-6 text-red-500">Identity not found</div>;

  return (
    <div className="container mx-auto p-6">
      <Link
        href="/agent-identities"
        className="mb-6 inline-flex items-center text-sm text-muted-foreground hover:text-foreground"
      >
        <ArrowLeft className="mr-2 h-4 w-4" />
        Back to Agent Identities
      </Link>
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            {identity.name}
            <CopyButton value={identity.id} />
            <Badge variant={getEntityStatusBadgeVariant(identity.status)}>{identity.status}</Badge>
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-4 md:grid-cols-2">
            <Field
              label="Name"
              value={identity.name}
              onSave={(value) =>
                updateIdentity.mutateAsync({ identityId, request: { name: value } })
              }
            />
            <Field
              label="Description"
              value={identity.description ?? ""}
              onSave={(value) =>
                updateIdentity.mutateAsync({ identityId, request: { description: value || null } })
              }
            />
            <Field
              label="Locale"
              value={identity.locale ?? ""}
              onSave={(value) =>
                updateIdentity.mutateAsync({ identityId, request: { locale: value || null } })
              }
            />
            <Field
              label="Timezone"
              value={identity.timezone ?? ""}
              onSave={(value) =>
                updateIdentity.mutateAsync({ identityId, request: { timezone: value || null } })
              }
            />
          </div>
          <div className="flex gap-3">
            {identity.status === "active" ? (
              <Button
                variant="outline"
                onClick={async () => {
                  await deleteIdentity.mutateAsync(identityId);
                  router.refresh();
                }}
              >
                Archive
              </Button>
            ) : identity.status === "archived" ? (
              <Button
                variant="destructive"
                onClick={async () => {
                  await destroyIdentity.mutateAsync(identityId);
                  router.push("/agent-identities");
                }}
              >
                <Trash2 className="mr-2 h-4 w-4" />
                Delete
              </Button>
            ) : null}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

function Field({
  label,
  value,
  onSave,
}: {
  label: string;
  value: string;
  onSave: (value: string) => Promise<unknown>;
}) {
  return (
    <div className="space-y-2">
      <Label>{label}</Label>
      <div className="flex gap-2">
        <Input
          defaultValue={value}
          onBlur={(e) => {
            void onSave(e.target.value).catch(() => {});
          }}
        />
      </div>
    </div>
  );
}
