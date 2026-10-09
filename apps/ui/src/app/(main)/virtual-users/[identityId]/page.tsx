"use client";

import { VirtualUserIcon } from "@/components/icons/facet-icons";
import { use, useState, useMemo, useCallback } from "react";
import { useRouter } from "next/navigation";
import { ArchiveRestore, Check, Pencil } from "lucide-react";
import {
  useVirtualUser,
  useDeleteVirtualUser,
  useDestroyVirtualUser,
  useUpdateVirtualUser,
} from "@/hooks/use-virtual-users";
import { usePageTitle } from "@/hooks";
import { usePolicies } from "@/hooks/use-policies";
import { EntityActionsMenu } from "@/components/entity-actions/entity-actions-menu";
import { ResourceNotFound } from "@/components/resource-not-found";
import { EntityDeleteErrorNotice } from "@/components/entity-delete-error-notice";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
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
import { LinkedIdentities } from "@/components/virtual-user/linked-identities";
import { VirtualUserSessions } from "@/components/virtual-user/virtual-user-sessions";
import { SectionTabs } from "@/components/layout";
import { IdentityConnections } from "@/components/virtual-user/identity-connections";

interface FormData {
  name: string;
  description: string;
  locale: string;
  timezone: string;
}

export default function VirtualUserDetailPage({
  params,
}: {
  params: Promise<{ identityId: string }>;
}) {
  const { identityId } = use(params);
  const router = useRouter();
  const [tab, setTab] = useState("overview");
  const { data: identity, isLoading } = useVirtualUser(identityId);
  usePageTitle(identity?.name ?? null, "Virtual User");
  const updateIdentity = useUpdateVirtualUser();
  const deleteIdentity = useDeleteVirtualUser();
  const destroyIdentity = useDestroyVirtualUser();
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  // Virtual users expose no policy config; their manage permission is granted
  // to exactly the roles that hold agent manage, so that answer stands in.
  const { can } = usePolicies("agents");

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
      router.push("/virtual-users");
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
        title="Virtual user not found"
        description="This virtual user may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/virtual-users"
        backLabel="Back to virtual users"
        resourceId={identityId}
      />
    );
  }

  return (
    <PageContainer>
      <PageBreadcrumb
        items={[{ label: "Virtual Users", href: "/virtual-users" }, { label: identity.name }]}
      />

      <PageMasthead
        icon={<VirtualUserIcon />}
        entityId={identity.id}
        title={<span className={getEntityNameClassName(identity.status)}>{identity.name}</span>}
        badges={
          <>
            <Badge variant={getEntityStatusBadgeVariant(identity.status)}>{identity.status}</Badge>
            <Badge variant="outline">
              {identity.usage === "end_user" ? "End user" : "Service"}
            </Badge>
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
            <EntityActionsMenu
              entityRef={identity.id}
              kind="virtual_user"
              entityName={identity.name}
              permissions={{ manage: can("agent.manage") }}
              actions={
                identity.status === "archived"
                  ? [
                      {
                        id: "unarchive",
                        label: "Unarchive virtual user",
                        icon: <ArchiveRestore className="size-4" />,
                        disabled: updateIdentity.isPending,
                        onSelect: () =>
                          updateIdentity.mutate({ identityId, request: { status: "active" } }),
                      },
                    ]
                  : []
              }
              archive={
                identity.status === "archived"
                  ? undefined
                  : {
                      onSelect: handleArchive,
                      label: deleteIdentity.isPending ? "Archiving..." : "Archive virtual user",
                      disabled: deleteIdentity.isPending,
                    }
              }
              delete={
                identity.status === "archived"
                  ? {
                      onSelect: () => setShowDeleteDialog(true),
                      disabled: destroyIdentity.isPending,
                    }
                  : undefined
              }
            />
          </>
        }
      />

      {deleteError && (
        <EntityDeleteErrorNotice
          entityKind="identity"
          action={deleteAction}
          message={deleteError.message}
        />
      )}

      <SectionTabs
        value={tab}
        onValueChange={setTab}
        items={[
          { value: "overview", label: "Overview" },
          { value: "connections", label: "Connections" },
          { value: "bindings", label: "Linked identities" },
          { value: "sessions", label: "Sessions" },
        ]}
      />
      {tab === "bindings" && (
        <Card>
          <CardHeader>
            <CardTitle>Linked identities</CardTitle>
          </CardHeader>
          <CardContent>
            <LinkedIdentities identityId={identityId} />
          </CardContent>
        </Card>
      )}
      {tab === "sessions" && (
        <Card>
          <CardHeader>
            <CardTitle>Sessions</CardTitle>
          </CardHeader>
          <CardContent>
            <VirtualUserSessions identityId={identityId} />
          </CardContent>
        </Card>
      )}
      {tab === "connections" && (
        <Card>
          <CardContent className="pt-6">
            <IdentityConnections identityId={identityId} />
          </CardContent>
        </Card>
      )}
      <form
        className={tab !== "overview" ? "hidden" : undefined}
        id="identity-edit-form"
        onSubmit={handleSubmit}
      >
        <PageColumns>
          {/* Main form */}
          <PageMain>
            <Card>
              <CardHeader>
                <CardTitle>Profile</CardTitle>
              </CardHeader>
              <CardContent className="space-y-6">
                <div className="space-y-2">
                  <Label htmlFor="name">Name</Label>
                  <Input
                    id="name"
                    placeholder="Virtual user name"
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
                    placeholder="Describe this virtual user..."
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
        <BackLink href="/virtual-users">Back to Virtual Users</BackLink>
      </PageFooter>

      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Delete Virtual User</DialogTitle>
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
              {destroyIdentity.isPending ? "Deleting..." : "Delete Virtual User"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </PageContainer>
  );
}
