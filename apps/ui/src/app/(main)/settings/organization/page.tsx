"use client";

import { useEffect, useRef, useState } from "react";
import { useRouter } from "next/navigation";
import { AlertCircle, Building2, Check, Loader2, Plus } from "lucide-react";
import { ModelPicker } from "@/components/models/model-picker";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { CopyButton } from "@/components/ui/copy-button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { HarnessSelect } from "@/components/harness/harness-select";
import {
  useCreateOrganization,
  useOrganization,
  useUpdateOrganization,
} from "@/hooks/use-organizations";
import { useHarnesses, usePageTitle } from "@/hooks";
import { useOrg } from "@/providers/org-provider";
import { cn } from "@/lib/utils";
import type { OrgRole } from "@/lib/api/types";

const ROLE_LABELS: Record<OrgRole, string> = {
  owner: "Owner",
  admin: "Admin",
  member: "Member",
};

type SettingsGroupKey = "identity" | "models" | "harnesses";
type AutoSaveState = "idle" | "dirty" | "saving" | "saved" | "error";

interface OrganizationDraft {
  name: string;
  defaultModelId: string;
  defaultHarnessId: string;
  baseHarnessId: string;
}

const CONTROL_CLASS_NAME = "w-full";
const AUTOSAVE_DELAY_MS = 700;

export default function OrganizationPage() {
  usePageTitle("Organisation", "Settings");
  const router = useRouter();
  const { currentOrg, organizations, setCurrentOrg } = useOrg();
  const { data: organization, isLoading, error } = useOrganization();
  const { data: harnesses = [] } = useHarnesses();
  const updateOrganization = useUpdateOrganization();
  const createOrganization = useCreateOrganization();
  const genericHarnessId = harnesses.find((h) => h.name === "Generic")?.id || "";
  const baseHarnessFallbackId = harnesses.find((h) => h.name === "Base")?.id || "";
  const organizationId = organization?.id;
  const organizationName = organization?.name;
  const organizationDefaultModelId = organization?.default_model_id || "";
  const organizationDefaultHarnessId = organization?.default_harness_id || genericHarnessId;
  const organizationBaseHarnessId = organization?.base_harness_id || baseHarnessFallbackId;

  const [name, setName] = useState("");
  const [defaultModelId, setDefaultModelId] = useState("");
  const [defaultHarnessId, setDefaultHarnessId] = useState("");
  const [baseHarnessId, setBaseHarnessId] = useState("");
  const [dirtyGroups, setDirtyGroups] = useState<SettingsGroupKey[]>([]);
  const [savingGroups, setSavingGroups] = useState<SettingsGroupKey[]>([]);
  const [recentlySavedGroups, setRecentlySavedGroups] = useState<SettingsGroupKey[]>([]);
  const [autoSaveState, setAutoSaveState] = useState<AutoSaveState>("idle");
  const [autoSaveError, setAutoSaveError] = useState<string | null>(null);
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [newOrgName, setNewOrgName] = useState("");

  const draftRef = useRef<OrganizationDraft>({
    name: "",
    defaultModelId: "",
    defaultHarnessId: "",
    baseHarnessId: "",
  });
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const saveVersionRef = useRef(0);

  useEffect(() => {
    if (saveTimerRef.current) {
      clearTimeout(saveTimerRef.current);
      saveTimerRef.current = null;
    }
    saveVersionRef.current += 1;

    if (!organizationId || !organizationName) {
      return;
    }

    setName(organizationName);
    setDefaultModelId(organizationDefaultModelId);
    setDefaultHarnessId(organizationDefaultHarnessId);
    setBaseHarnessId(organizationBaseHarnessId);
    const draft = {
      name: organizationName,
      defaultModelId: organizationDefaultModelId,
      defaultHarnessId: organizationDefaultHarnessId,
      baseHarnessId: organizationBaseHarnessId,
    };

    draftRef.current = draft;
    setDirtyGroups([]);
    setSavingGroups([]);
    setRecentlySavedGroups([]);
    setAutoSaveState("idle");
    setAutoSaveError(null);
  }, [
    organizationId,
    organizationName,
    organizationDefaultModelId,
    organizationDefaultHarnessId,
    organizationBaseHarnessId,
  ]);

  useEffect(() => {
    return () => {
      if (saveTimerRef.current) {
        clearTimeout(saveTimerRef.current);
        saveTimerRef.current = null;
      }
      saveVersionRef.current += 1;
    };
  }, []);

  const setDraft = (nextDraft: OrganizationDraft) => {
    draftRef.current = nextDraft;
    setName(nextDraft.name);
    setDefaultModelId(nextDraft.defaultModelId);
    setDefaultHarnessId(nextDraft.defaultHarnessId);
    setBaseHarnessId(nextDraft.baseHarnessId);
  };

  const scheduleAutoSave = (groups: SettingsGroupKey[], nextDraft: OrganizationDraft) => {
    draftRef.current = nextDraft;
    saveVersionRef.current += 1;
    const saveVersion = saveVersionRef.current;

    setRecentlySavedGroups([]);
    setDirtyGroups((current) => mergeGroups(current, groups));
    setAutoSaveError(null);

    if (saveTimerRef.current) {
      clearTimeout(saveTimerRef.current);
    }

    if (!nextDraft.name.trim() || !nextDraft.defaultHarnessId || !nextDraft.baseHarnessId) {
      setAutoSaveState("dirty");
      return;
    }

    setAutoSaveState("dirty");
    saveTimerRef.current = setTimeout(async () => {
      saveTimerRef.current = null;
      setAutoSaveState("saving");
      setSavingGroups(groups);

      try {
        await updateOrganization.mutateAsync({
          name: nextDraft.name.trim(),
          ...(nextDraft.defaultModelId ? { default_model_id: nextDraft.defaultModelId } : {}),
          default_harness_id: nextDraft.defaultHarnessId,
          base_harness_id: nextDraft.baseHarnessId,
        });

        if (saveVersionRef.current !== saveVersion) {
          return;
        }

        const savedDraft = {
          ...nextDraft,
          name: nextDraft.name.trim(),
        };

        setDraft(savedDraft);
        setDirtyGroups([]);
        setSavingGroups([]);
        setRecentlySavedGroups(groups);
        setAutoSaveState("saved");
      } catch (err) {
        if (saveVersionRef.current !== saveVersion) {
          return;
        }

        setSavingGroups([]);
        setAutoSaveState("error");
        setAutoSaveError(err instanceof Error ? err.message : "Failed to save organisation");
      }
    }, AUTOSAVE_DELAY_MS);
  };

  const handleNameChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const nextName = e.target.value;
    const nextDraft = { ...draftRef.current, name: nextName };
    setName(nextName);
    scheduleAutoSave(["identity"], nextDraft);
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" && saveTimerRef.current) {
      clearTimeout(saveTimerRef.current);
      saveTimerRef.current = null;
      scheduleAutoSave(["identity"], draftRef.current);
    }
  };

  const handleCreateOrg = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!newOrgName.trim()) return;

    const org = await createOrganization.mutateAsync({
      name: newOrgName.trim(),
    });
    setCurrentOrg({ public_id: org.id, name: org.name, role: "owner" });
    setNewOrgName("");
    setCreateDialogOpen(false);
    router.push(`/orgs/${org.id}/setup`);
  };

  return (
    <div className="space-y-8">
      <section>
        <div className="mb-6">
          <h2 className="text-xl font-semibold">Organisation</h2>
          <p className="text-sm text-muted-foreground">
            Manage organisation-level defaults and identity.
          </p>
        </div>

        {error ? (
          <Card className="p-8 text-center">
            <AlertCircle className="h-12 w-12 mx-auto text-destructive mb-4" />
            <h3 className="text-lg font-medium mb-2">Failed to load organisation</h3>
            <p className="text-muted-foreground">{error.message}</p>
          </Card>
        ) : isLoading ? (
          <Card className="p-6">
            <div className="space-y-6">
              <div className="space-y-2">
                <Skeleton className="h-4 w-24" />
                <Skeleton className="h-10 w-full" />
              </div>
              <div className="space-y-2">
                <Skeleton className="h-4 w-32" />
                <Skeleton className="h-10 w-full" />
              </div>
              <div className="space-y-2">
                <Skeleton className="h-4 w-28" />
                <Skeleton className="h-10 w-full" />
              </div>
            </div>
          </Card>
        ) : (
          <div className="space-y-6">
            <SettingsGroup
              title="Identity"
              description="Name and stable identifier for this organisation."
              status={
                <AutoSaveBadge
                  group="identity"
                  state={autoSaveState}
                  dirtyGroups={dirtyGroups}
                  savingGroups={savingGroups}
                  recentlySavedGroups={recentlySavedGroups}
                />
              }
            >
              <SettingsRow
                label="Organisation Name"
                description="Shown in navigation, resource ownership, and member-facing views."
                htmlFor="org-name"
              >
                <Input
                  id="org-name"
                  value={name}
                  onChange={handleNameChange}
                  onKeyDown={handleKeyDown}
                  placeholder="Organisation name"
                  className={CONTROL_CLASS_NAME}
                />
              </SettingsRow>

              <SettingsRow
                label="Organisation ID"
                description="Use this ID when making API calls scoped to this organisation."
                htmlFor="org-id"
              >
                <div className="flex items-center gap-2">
                  <Input
                    id="org-id"
                    value={organization?.id || currentOrg?.public_id || ""}
                    readOnly
                    className="w-full font-mono text-sm bg-muted"
                  />
                  <CopyButton value={organization?.id || currentOrg?.public_id || ""} />
                </div>
              </SettingsRow>
            </SettingsGroup>

            <SettingsGroup
              title="Models"
              description="Model fallbacks applied when agents or sessions do not set their own model."
              status={
                <AutoSaveBadge
                  group="models"
                  state={autoSaveState}
                  dirtyGroups={dirtyGroups}
                  savingGroups={savingGroups}
                  recentlySavedGroups={recentlySavedGroups}
                />
              }
            >
              <SettingsRow
                label="Default Model"
                description="Used when no model is specified at the agent or session level."
                htmlFor="default-model"
              >
                <ModelPicker
                  id="default-model"
                  value={defaultModelId}
                  onChange={(value) => {
                    const nextDraft = { ...draftRef.current, defaultModelId: value };
                    setDefaultModelId(value);
                    scheduleAutoSave(["models"], nextDraft);
                  }}
                  placeholder="No default model"
                  className={CONTROL_CLASS_NAME}
                />
              </SettingsRow>
            </SettingsGroup>

            <SettingsGroup
              title="Harnesses"
              description="Harness fallbacks applied when sessions do not set their own runtime environment."
              status={
                <AutoSaveBadge
                  group="harnesses"
                  state={autoSaveState}
                  dirtyGroups={dirtyGroups}
                  savingGroups={savingGroups}
                  recentlySavedGroups={recentlySavedGroups}
                />
              }
            >
              <SettingsRow
                label="Default Harness"
                description="Used as the normal starting harness in the UI. New orgs default this to Generic."
                htmlFor="default-harness"
              >
                <HarnessSelect
                  id="default-harness"
                  value={defaultHarnessId}
                  onValueChange={(value) => {
                    const nextDraft = { ...draftRef.current, defaultHarnessId: value };
                    setDefaultHarnessId(value);
                    scheduleAutoSave(["harnesses"], nextDraft);
                  }}
                  placeholder="Select default harness"
                  className={CONTROL_CLASS_NAME}
                />
              </SettingsRow>

              <SettingsRow
                label="Base Harness"
                description="Used only when a session starts without an explicit harness. New orgs default this to Base."
                htmlFor="base-harness"
              >
                <HarnessSelect
                  id="base-harness"
                  value={baseHarnessId}
                  onValueChange={(value) => {
                    const nextDraft = { ...draftRef.current, baseHarnessId: value };
                    setBaseHarnessId(value);
                    scheduleAutoSave(["harnesses"], nextDraft);
                  }}
                  placeholder="Select base harness"
                  className={CONTROL_CLASS_NAME}
                />
              </SettingsRow>

              {harnesses.length === 0 && (
                <div className="px-5 pb-4 text-xs text-muted-foreground">
                  Harnesses will appear after org init.
                </div>
              )}
            </SettingsGroup>

            {(autoSaveState === "error" || autoSaveError) && (
              <p className="text-sm text-destructive">{autoSaveError}</p>
            )}
          </div>
        )}
      </section>

      <section>
        <div className="mb-6 flex flex-col items-stretch gap-3 xl:flex-row xl:items-center xl:justify-between">
          <div>
            <h2 className="text-xl font-semibold">Organisations</h2>
            <p className="text-sm text-muted-foreground">All organisations you are a member of.</p>
          </div>
          <Button className="w-full xl:w-auto" onClick={() => setCreateDialogOpen(true)}>
            <Plus className="h-4 w-4 mr-2" />
            Create Organisation
          </Button>
        </div>

        {organizations.length === 0 ? (
          <Card className="p-8 text-center">
            <Building2 className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">No organisations</h3>
            <p className="text-muted-foreground">
              Create an organisation to start managing shared resources.
            </p>
          </Card>
        ) : (
          <Card className="overflow-hidden">
            <div className="overflow-x-auto">
              <Table aria-label="Organisations" className="min-w-[42rem]">
                <TableHeader>
                  <TableRow>
                    <TableHead>Organisation</TableHead>
                    <TableHead>ID</TableHead>
                    <TableHead>Role</TableHead>
                    <TableHead>Status</TableHead>
                    <TableHead className="text-right">Actions</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {organizations.map((org) => {
                    const isCurrent = currentOrg?.public_id === org.public_id;
                    return (
                      <TableRow key={org.public_id} data-state={isCurrent ? "selected" : undefined}>
                        <TableCell className="font-medium">{org.name}</TableCell>
                        <TableCell className="font-mono text-xs text-muted-foreground">
                          {org.public_id}
                        </TableCell>
                        <TableCell>{ROLE_LABELS[org.role]}</TableCell>
                        <TableCell>
                          {isCurrent ? (
                            <Badge variant="accent">
                              <Check className="h-3 w-3" />
                              Current
                            </Badge>
                          ) : (
                            <span className="text-muted-foreground">Available</span>
                          )}
                        </TableCell>
                        <TableCell className="text-right">
                          <Button
                            variant="outline"
                            size="sm"
                            disabled={isCurrent}
                            aria-label={`Switch to ${org.name}`}
                            onClick={() => setCurrentOrg(org)}
                          >
                            Switch
                          </Button>
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </div>
          </Card>
        )}
      </section>

      <Dialog open={createDialogOpen} onOpenChange={setCreateDialogOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Create Organisation</DialogTitle>
            <DialogDescription>
              Create a new organisation. You will be added as a member automatically.
            </DialogDescription>
          </DialogHeader>
          <form onSubmit={handleCreateOrg} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="new-org-name">Name</Label>
              <Input
                id="new-org-name"
                value={newOrgName}
                onChange={(e) => setNewOrgName(e.target.value)}
                placeholder="Organisation name"
                required
              />
            </div>
            {createOrganization.isError && (
              <p className="text-sm text-destructive">
                Failed to create: {createOrganization.error.message}
              </p>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => setCreateDialogOpen(false)}>
                Cancel
              </Button>
              <Button type="submit" disabled={createOrganization.isPending || !newOrgName.trim()}>
                {createOrganization.isPending ? "Creating..." : "Create"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </div>
  );
}

function SettingsGroup({
  title,
  description,
  status,
  children,
}: {
  title: string;
  description: string;
  status: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <div>
      <div className="mb-3 flex items-start justify-between gap-4">
        <div>
          <h3 className="text-sm font-semibold uppercase tracking-[0.08em] text-muted-foreground">
            {title}
          </h3>
          <p className="mt-1 text-sm text-muted-foreground">{description}</p>
        </div>
        {status}
      </div>
      <Card className="overflow-hidden p-0 gap-0">{children}</Card>
    </div>
  );
}

function SettingsRow({
  label,
  description,
  htmlFor,
  children,
}: {
  label: string;
  description: string;
  htmlFor: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid gap-3 border-b px-5 py-4 last:border-b-0 xl:grid-cols-[minmax(0,1fr)_minmax(18rem,24rem)] xl:items-center">
      <div className="min-w-0">
        <Label htmlFor={htmlFor} className="text-sm font-medium">
          {label}
        </Label>
        <p className="mt-1 text-sm leading-5 text-muted-foreground">{description}</p>
      </div>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

function AutoSaveBadge({
  group,
  state,
  dirtyGroups,
  savingGroups,
  recentlySavedGroups,
}: {
  group: SettingsGroupKey;
  state: AutoSaveState;
  dirtyGroups: SettingsGroupKey[];
  savingGroups: SettingsGroupKey[];
  recentlySavedGroups: SettingsGroupKey[];
}) {
  const isSaving = state === "saving" && savingGroups.includes(group);
  const isDirty = dirtyGroups.includes(group);
  const isRecentlySaved = state === "saved" && recentlySavedGroups.includes(group);
  const isError = state === "error" && isDirty;

  if (isSaving) {
    return (
      <Badge variant="outline" className="text-muted-foreground">
        <Loader2 className="h-3 w-3 animate-spin" />
        Saving
      </Badge>
    );
  }

  if (isError) {
    return <Badge variant="destructive">Save failed</Badge>;
  }

  if (isDirty) {
    return <Badge variant="secondary">Unsaved changes</Badge>;
  }

  return (
    <Badge variant="outline" className={cn(isRecentlySaved && "text-green-700")}>
      <Check className="h-3 w-3" />
      Saved
    </Badge>
  );
}

function mergeGroups(
  current: SettingsGroupKey[],
  incoming: SettingsGroupKey[],
): SettingsGroupKey[] {
  return Array.from(new Set([...current, ...incoming]));
}
