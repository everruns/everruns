"use client";

import { use, useState, useMemo, useCallback } from "react";
import { useRouter } from "next/navigation";
import { Check, Pencil, Trash2, UserRound } from "lucide-react";
import {
  useAgentIdentity,
  useDeleteAgentIdentity,
  useDestroyAgentIdentity,
  useUpdateAgentIdentity,
} from "@/hooks/use-agent-identities";
import { usePageTitle } from "@/hooks";
import { ResourceNotFound } from "@/components/resource-not-found";
import { EntityDeleteErrorNotice } from "@/components/entity-delete-error-notice";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Combobox } from "@/components/ui/combobox";
import { PromptEditor } from "@/components/ui/prompt-editor";
import {
  PageContainer,
  PageBreadcrumb,
  PageMasthead,
  PageColumns,
  PageMain,
  PageRail,
  PageFooter,
  BackLink,
} from "@/components/layout";
import {
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { LOCALE_OPTIONS, TIMEZONE_OPTIONS } from "@/lib/locale-data";
import { IdentityConnections } from "@/components/agent-identity/identity-connections";

interface FormData {
  name: string;
  description: string;
  locale: string;
  timezone: string;
}

export default function AgentIdentityDetailPage({
  params,
}: {
  params: Promise<{ identityId: string }>;
}) {
  const { identityId } = use(params);
  const router = useRouter();
  const { data: identity, isLoading } = useAgentIdentity(identityId);
  usePageTitle(identity?.name ?? null, "Agent Identity");
  const updateIdentity = useUpdateAgentIdentity();
  const deleteIdentity = useDeleteAgentIdentity();
  const destroyIdentity = useDestroyAgentIdentity();
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);

  // Form state - track user changes separately from initial values
  const [formChanges, setFormChanges] = useState<Partial<FormData>>({});

  const initialFormData = useMemo((): FormData => {
    if (!identity) return { name: "", description: "", locale: "", timezone: "" };
    return {
      name: identity.name,
      description: identity.description || "",
      locale: identity.locale || "",
      timezone: identity.timezone || "",
    };
  }, [identity]);

  const formData = useMemo(
    () => ({ ...initialFormData, ...formChanges }),
    [initialFormData, formChanges],
  );

  const handleFormChange = useCallback((field: keyof FormData, value: string) => {
    setFormChanges((prev) => ({ ...prev, [field]: value }));
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      await updateIdentity.mutateAsync({
        identityId,
        request: {
          name: formData.name,
          description: formData.description || null,
          locale: formData.locale || null,
          timezone: formData.timezone || null,
        },
      });
      setFormChanges({});
    } catch (error) {
      console.error("Failed to update identity:", error);
    }
  };

  const handleArchive = async () => {
    try {
      await deleteIdentity.mutateAsync(identityId);
    } catch (error) {
      console.error("Failed to archive identity:", error);
    }
  };

  const handleDestroy = async () => {
    try {
      await destroyIdentity.mutateAsync(identityId);
      router.push("/agent-identities");
    } catch (error) {
      console.error("Failed to delete identity:", error);
    }
  };

  const isSaving = updateIdentity.isPending;
  const isReadOnly = isReadOnlyStatus(identity?.status);
  const deleteError = deleteIdentity.error ?? destroyIdentity.error;
  const deleteAction = identity?.status === "archived" ? "delete" : "archive";

  if (isLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="h-8 w-1/4 mb-6" />
        <Skeleton className="h-[400px] w-full" />
      </div>
    );
  }

  if (!identity) {
    return (
      <ResourceNotFound
        title="Identity not found"
        description="This agent identity may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/agent-identities"
        backLabel="Back to agent identities"
        resourceId={identityId}
      />
    );
  }

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: "Agent Identities", href: "/agent-identities" }, { label: identity.name }]}
      />

      <PageMasthead
        icon={<UserRound />}
        entityId={identity.id}
        title={<span className={getEntityNameClassName(identity.status)}>{identity.name}</span>}
        badges={
          <>
            <Badge variant={getEntityStatusBadgeVariant(identity.status)}>{identity.status}</Badge>
            {!isReadOnly && (
              <Badge variant="accent">
                <Pencil className="size-3" />
                Editing
              </Badge>
            )}
          </>
        }
        description={identity.description || undefined}
        meta={
          <>
            <span>
              Locale <span className="text-primary">{identity.locale || "—"}</span>
            </span>
            <span>
              Timezone <span className="text-primary">{identity.timezone || "—"}</span>
            </span>
            <span>
              Created{" "}
              <span className="text-foreground">
                {new Date(identity.created_at).toLocaleDateString()}
              </span>
            </span>
          </>
        }
        actions={
          <>
            <Button type="submit" form="identity-edit-form" disabled={isSaving || isReadOnly}>
              <Check className="size-4" />
              {isReadOnly ? "Read-only" : isSaving ? "Saving..." : "Save changes"}
            </Button>
            <Button type="button" variant="outline" onClick={() => router.back()}>
              Discard
            </Button>
          </>
        }
      />

      <form id="identity-edit-form" onSubmit={handleSubmit}>
        <PageColumns>
          {/* Main form */}
          <PageMain>
            <Card>
              <CardHeader>
                <CardTitle>Identity Details</CardTitle>
              </CardHeader>
              <CardContent className="space-y-6">
                <div className="space-y-2">
                  <Label htmlFor="name">Name</Label>
                  <Input
                    id="name"
                    placeholder="Identity name"
                    value={formData.name}
                    onChange={(e) => handleFormChange("name", e.target.value)}
                    disabled={isSaving || isReadOnly}
                    required
                  />
                </div>

                <div className="space-y-2">
                  <Label htmlFor="description">Description</Label>
                  <PromptEditor
                    id="description"
                    placeholder="Describe this identity..."
                    value={formData.description}
                    onChange={(value) => handleFormChange("description", value)}
                    disabled={isSaving || isReadOnly}
                  />
                  <p className="text-xs text-muted-foreground">Supports Markdown</p>
                </div>

                <div className="grid gap-4 md:grid-cols-2">
                  <div className="space-y-2">
                    <Label>Locale</Label>
                    <Combobox
                      options={LOCALE_OPTIONS}
                      value={formData.locale}
                      onValueChange={(value) => handleFormChange("locale", value)}
                      placeholder="Select locale..."
                      searchPlaceholder="Search locales..."
                      disabled={isSaving || isReadOnly}
                    />
                    <p className="text-xs text-muted-foreground">
                      BCP 47 locale for formatting and translations
                    </p>
                  </div>
                  <div className="space-y-2">
                    <Label>Timezone</Label>
                    <Combobox
                      options={TIMEZONE_OPTIONS}
                      value={formData.timezone}
                      onValueChange={(value) => handleFormChange("timezone", value)}
                      placeholder="Select timezone..."
                      searchPlaceholder="Search timezones..."
                      disabled={isSaving || isReadOnly}
                    />
                    <p className="text-xs text-muted-foreground">
                      IANA timezone for date/time rendering
                    </p>
                  </div>
                </div>
              </CardContent>
            </Card>

            {/* Connections */}
            <Card>
              <CardHeader>
                <CardTitle>Connections</CardTitle>
                <CardDescription>
                  Connections assigned to this identity are used in sessions running as this
                  identity
                </CardDescription>
              </CardHeader>
              <CardContent>
                <IdentityConnections identityId={identityId} />
              </CardContent>
            </Card>

            {/* Danger Zone */}
            <Card className="border-destructive/50">
              <CardHeader>
                <CardTitle className="text-destructive">Danger Zone</CardTitle>
                <CardDescription>Irreversible actions that affect this identity</CardDescription>
              </CardHeader>
              <CardContent>
                <div className="flex items-center justify-between">
                  <div>
                    <p className="font-medium">
                      {identity.status === "archived"
                        ? "Delete this identity"
                        : "Archive this identity"}
                    </p>
                    <p className="text-sm text-muted-foreground">
                      {identity.status === "archived"
                        ? "Permanently delete this archived identity. Existing references will render as deleted tombstones."
                        : "Archive this identity. It will stay visible when archived items are shown, become read-only, and stop being assignable."}
                    </p>
                  </div>
                  {identity.status === "archived" ? (
                    <Button
                      type="button"
                      variant="destructive"
                      onClick={() => setShowDeleteDialog(true)}
                      disabled={destroyIdentity.isPending}
                    >
                      <Trash2 className="w-4 h-4 mr-2" />
                      {destroyIdentity.isPending ? "Deleting..." : "Delete Identity"}
                    </Button>
                  ) : (
                    <Button
                      type="button"
                      variant="outline"
                      onClick={handleArchive}
                      disabled={deleteIdentity.isPending}
                    >
                      {deleteIdentity.isPending ? "Archiving..." : "Archive Identity"}
                    </Button>
                  )}
                </div>
                {deleteError && (
                  <EntityDeleteErrorNotice
                    entityKind="identity"
                    action={deleteAction}
                    message={deleteError.message}
                    className="mt-4"
                  />
                )}
              </CardContent>
            </Card>
          </PageMain>

          {/* Summary sidebar */}
          <PageRail>
            {updateIdentity.error && (
              <p className="text-sm text-destructive">Error: {updateIdentity.error.message}</p>
            )}

            <Card>
              <CardHeader>
                <CardTitle>Summary</CardTitle>
              </CardHeader>
              <CardContent className="space-y-4">
                <div>
                  <p className="text-sm font-medium">Status</p>
                  <p className="text-sm text-muted-foreground">{identity.status}</p>
                </div>
                <div>
                  <p className="text-sm font-medium">Locale</p>
                  <p className="text-sm text-muted-foreground">{formData.locale || "(not set)"}</p>
                </div>
                <div>
                  <p className="text-sm font-medium">Timezone</p>
                  <p className="text-sm text-muted-foreground">
                    {formData.timezone || "(not set)"}
                  </p>
                </div>
                <div>
                  <p className="text-sm font-medium">Created</p>
                  <p className="text-sm text-muted-foreground">
                    {new Date(identity.created_at).toLocaleDateString()}
                  </p>
                </div>
                <div>
                  <p className="text-sm font-medium">Updated</p>
                  <p className="text-sm text-muted-foreground">
                    {new Date(identity.updated_at).toLocaleDateString()}
                  </p>
                </div>
              </CardContent>
            </Card>
          </PageRail>
        </PageColumns>
      </form>

      <PageFooter>
        <BackLink href="/agent-identities">Back to Agent Identities</BackLink>
      </PageFooter>

      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Identity</DialogTitle>
            <DialogDescription>
              Permanently delete the archived identity &quot;{identity.name}&quot;? Existing
              references will render as deleted tombstones.
            </DialogDescription>
            {destroyIdentity.error && (
              <EntityDeleteErrorNotice
                entityKind="identity"
                action="delete"
                message={destroyIdentity.error.message}
                className="mt-4"
              />
            )}
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowDeleteDialog(false)}>
              Cancel
            </Button>
            <Button
              variant="destructive"
              onClick={handleDestroy}
              disabled={destroyIdentity.isPending}
            >
              {destroyIdentity.isPending ? "Deleting..." : "Delete Identity"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </PageContainer>
  );
}
