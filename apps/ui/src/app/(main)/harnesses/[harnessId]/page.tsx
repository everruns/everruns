"use client";

// Harness page: one layout for reading and editing a harness.
//
// Capabilities are the page (wide left pane). A harness is the capability set
// a session starts from; the system prompt is optional, so it is a More row
// like branding and files. A narrow config column holds parent, model, and
// tags. Edit mode is page-level and in place: the same panes turn writable,
// the header swaps Edit for Save changes / Discard, and one Save sends the
// whole draft. The old /edit route redirects here with `?mode=edit`.
//
// One tab row only: Harness · Preview · Integrate · Stats. Archive and delete
// live in the header overflow menu. Built-in harnesses stay read-only.

import { use, useCallback, useEffect, useMemo, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { Check, Copy, Pencil } from "lucide-react";
import {
  useCapabilities,
  useCopyHarness,
  useDeleteHarness,
  useDestroyHarness,
  useHarness,
  useHarnesses,
  useHarnessStats,
  useModels,
  usePageTitle,
  useUpdateHarness,
} from "@/hooks";
import { usePolicies } from "@/hooks/use-policies";
import { ResourceNotFound } from "@/components/resource-not-found";
import { EntityDeleteErrorNotice } from "@/components/entity-delete-error-notice";
import { EntityActionsMenu } from "@/components/entity-actions/entity-actions-menu";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  BackLink,
  PageBreadcrumb,
  PageContainer,
  PageControlStrip,
  PageFooter,
  PageMasthead,
  SectionTabs,
} from "@/components/layout";
import { promptStats } from "@/components/agents/agent-prompt-pane";
import { HarnessCapabilitiesPane } from "@/components/harnesses/harness-capabilities-pane";
import {
  HarnessConfigColumn,
  type HarnessMoreRow,
} from "@/components/harnesses/harness-config-column";
import { HarnessPreview } from "@/components/harnesses/harness-preview";
import {
  HarnessSettingsSheet,
  type HarnessSettingsSection,
} from "@/components/harnesses/harness-settings-sheet";
import {
  getHarnessTabItems,
  resolveHarnessTab,
  type HarnessTab,
} from "@/components/harnesses/harness-tabs";
import { BRANDING_FIELDS, useHarnessDraft } from "@/components/harnesses/use-harness-draft";
import { IntegrationGuide } from "@/components/integration/integration-guide";
import { ResourceStatsPanel } from "@/components/stats/resource-stats-panel";
import { normalizeNetworkAccess } from "@/components/network-access-editor";
import type { ModelWithProvider } from "@/lib/api/types";
import {
  getDisplayName,
  getEntityNameClassName,
  getEntityStatusBadgeVariant,
  isReadOnlyStatus,
} from "@/lib/entity-lifecycle";
import { pluralize } from "@/lib/formatting";
import { HarnessIcon } from "@/lib/harness-icons";

const count = (n: number, singular: string, plural?: string) =>
  `${n} ${pluralize(n, singular, plural)}`;

export default function HarnessDetailPage({ params }: { params: Promise<{ harnessId: string }> }) {
  const { harnessId } = use(params);
  const router = useRouter();
  const searchParams = useSearchParams();
  const [activeTab, setActiveTab] = useState<HarnessTab>(
    resolveHarnessTab(searchParams.get("tab")),
  );
  const [openSection, setOpenSection] = useState<HarnessSettingsSection | null>(null);
  const [editRequested, setEditRequested] = useState(() => searchParams.get("mode") === "edit");
  const [confirmAction, setConfirmAction] = useState<"archive" | "delete" | null>(null);

  const { data: harness, isLoading: harnessLoading } = useHarness(harnessId);
  usePageTitle(harness ? getDisplayName(harness) : null, "Harness");
  const { data: harnesses = [] } = useHarnesses();
  const { data: allCapabilities } = useCapabilities({ includeRetired: true });
  const { data: models } = useModels();
  const { data: stats, isLoading: statsLoading, error: statsError } = useHarnessStats(harnessId);
  const updateHarness = useUpdateHarness();
  const deleteHarness = useDeleteHarness();
  const destroyHarness = useDestroyHarness();
  const copyHarness = useCopyHarness();
  const { can } = usePolicies("harnesses");

  const draft = useHarnessDraft(harness);
  const readOnly = isReadOnlyStatus(harness?.status) || !!harness?.is_built_in;
  const editing = editRequested && !!harness && !readOnly;

  const modelMap = useMemo(
    () => new Map<string, ModelWithProvider>((models ?? []).map((m) => [m.id, m])),
    [models],
  );
  const defaultModel = harness?.default_model_id
    ? modelMap.get(harness.default_model_id)
    : undefined;
  const parentHarness = harness?.parent_harness_id
    ? harnesses.find((candidate) => candidate.id === harness.parent_harness_id)
    : undefined;

  useEffect(() => {
    if (!editing || !draft.isDirty) return;
    const warn = (event: BeforeUnloadEvent) => event.preventDefault();
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [editing, draft.isDirty]);

  const startEdit = useCallback(() => setEditRequested(true), []);
  const onDraftChange = useCallback(
    <T,>(apply: (value: T) => void) =>
      (value: T) => {
        apply(value);
        setEditRequested(true);
      },
    [],
  );

  const exitEdit = () => {
    setEditRequested(false);
    if (searchParams.get("mode") === "edit") router.replace(`/harnesses/${harnessId}`);
  };

  const handleDiscard = () => {
    draft.reset();
    updateHarness.reset();
    exitEdit();
  };

  const handleSave = async () => {
    const result = draft.buildRequest();
    if (!result.ok) {
      setActiveTab("harness");
      if (result.errors.system_prompt) setOpenSection("prompt");
      else if (BRANDING_FIELDS.some((field) => result.errors[field])) setOpenSection("branding");
      return;
    }
    try {
      await updateHarness.mutateAsync({ harnessId, request: result.request });
      draft.reset();
      exitEdit();
    } catch (error) {
      console.error("Failed to update harness:", error);
    }
  };

  const handleCopy = useCallback(async () => {
    try {
      const copied = await copyHarness.mutateAsync(harnessId);
      router.push(`/harnesses/${copied.id}`);
    } catch (error) {
      console.error("Failed to copy harness:", error);
    }
  }, [harnessId, copyHarness, router]);

  const handleConfirm = async () => {
    try {
      if (confirmAction === "archive") {
        await deleteHarness.mutateAsync(harnessId);
        setConfirmAction(null);
      } else if (confirmAction === "delete") {
        await destroyHarness.mutateAsync(harnessId);
        router.push("/harnesses");
      }
    } catch (error) {
      console.error(`Failed to ${confirmAction} harness:`, error);
    }
  };

  if (harnessLoading) {
    return (
      <div className="container mx-auto p-6">
        <Skeleton className="mb-4 h-8 w-1/3" />
        <Skeleton className="mb-8 h-4 w-2/3" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  if (!harness) {
    return (
      <ResourceNotFound
        title="Harness not found"
        description="This harness may have been deleted, moved to another organization, or the URL may be wrong."
        backHref="/harnesses"
        backLabel="Back to harnesses"
        resourceId={harnessId}
      />
    );
  }

  const isActive = harness.status === "active";
  const displayName = getDisplayName(harness);
  const network = normalizeNetworkAccess(draft.networkAccess);
  const brandingParts = [
    draft.fields.description.trim() ? "Description" : "No description",
    ...(draft.starters.length > 0 ? [count(draft.starters.length, "starter")] : []),
  ];
  const promptWords = promptStats(draft.fields.system_prompt).words;
  const moreRows: HarnessMoreRow[] = [
    {
      id: "prompt",
      label: "System prompt",
      summary: draft.fields.system_prompt.trim() ? count(promptWords, "word") : "None",
    },
    { id: "branding", label: "Branding", summary: brandingParts.join(" · ") },
    {
      id: "files",
      label: "Starter files",
      summary: draft.files.length ? count(draft.files.length, "file") : "None",
    },
    {
      id: "network",
      label: "Network access",
      summary:
        network.allowed.length || network.blocked.length
          ? [
              ...(network.allowed.length ? [`${network.allowed.length} allowed`] : []),
              ...(network.blocked.length ? [`${network.blocked.length} blocked`] : []),
            ].join(" · ")
          : "Unrestricted",
    },
    {
      id: "usage",
      label: "Usage",
      summary: `${count(harness.session_count ?? 0, "session")} · ${count(harness.app_count ?? 0, "app")}`,
    },
  ];

  const canArchive = isActive && !harness.is_built_in;
  const canDelete =
    harness.status === "archived" && !harness.is_built_in && can("harness.dangerous");
  const confirmError = confirmAction === "delete" ? destroyHarness.error : deleteHarness.error;
  const confirmPending = deleteHarness.isPending || destroyHarness.isPending;

  const overflowMenu = (
    <EntityActionsMenu
      entityRef={harness.id}
      kind="harness"
      entityName={displayName}
      permissions={{ manage: can("harness.manage") }}
      actions={[
        {
          id: "copy",
          label: copyHarness.isPending ? "Copying..." : "Copy",
          icon: <Copy className="size-4" />,
          onSelect: handleCopy,
          disabled: copyHarness.isPending,
        },
      ]}
      archive={canArchive ? { onSelect: () => setConfirmAction("archive") } : undefined}
      delete={canDelete ? { onSelect: () => setConfirmAction("delete") } : undefined}
    />
  );

  const actions = editing ? (
    <>
      <Button onClick={handleSave} disabled={updateHarness.isPending}>
        <Check className="size-4" />
        {updateHarness.isPending ? "Saving..." : "Save changes"}
      </Button>
      <Button variant="outline" onClick={handleDiscard} disabled={updateHarness.isPending}>
        Discard
      </Button>
    </>
  ) : (
    <>
      {isActive && !harness.is_built_in && (
        <Button variant="outline" onClick={startEdit}>
          <Pencil className="size-4" />
          Edit
        </Button>
      )}
      {overflowMenu}
    </>
  );

  return (
    <div className="min-h-full bg-brand-dots">
      <PageContainer>
        <PageBreadcrumb
          items={[{ label: "Harnesses", href: "/harnesses" }, { label: displayName }]}
        />

        <PageMasthead
          icon={<HarnessIcon icon={harness.icon} />}
          entityId={harness.id}
          title={<span className={getEntityNameClassName(harness.status)}>{displayName}</span>}
          badges={
            <>
              <span className="font-mono text-xs text-muted-foreground">{harness.name}</span>
              {harness.is_built_in && <Badge variant="outline">Built-in</Badge>}
              {editing ? (
                <Badge variant="accent">
                  <Pencil />
                  Editing
                </Badge>
              ) : (
                <Badge variant={getEntityStatusBadgeVariant(harness.status)}>
                  {harness.status}
                </Badge>
              )}
            </>
          }
          description={
            editing
              ? "Changes apply to new sessions only. Running sessions keep the current definition."
              : undefined
          }
          actions={actions}
        />

        {updateHarness.error && (
          <p role="alert" className="-mt-2 text-sm text-destructive">
            Could not save: {updateHarness.error.message}
          </p>
        )}

        <div className="flex min-w-0 flex-col">
          <PageControlStrip>
            <SectionTabs
              value={activeTab}
              onValueChange={(value) => setActiveTab(value as HarnessTab)}
              items={getHarnessTabItems()}
              className="border-x border-t bg-background px-2"
            />
          </PageControlStrip>

          {activeTab === "harness" && (
            <div className="grid min-w-0 border-x border-b bg-background lg:grid-cols-[minmax(0,1fr)_340px]">
              <HarnessCapabilitiesPane
                className="lg:border-r"
                draft={draft}
                editing={editing}
                onEdit={isActive && !harness.is_built_in ? startEdit : undefined}
                allCapabilities={allCapabilities ?? []}
                hasParent={!!harness.parent_harness_id}
              />
              <HarnessConfigColumn
                className="border-t lg:border-t-0"
                harness={harness}
                parentHarness={parentHarness}
                draft={draft}
                editing={editing}
                readOnly={readOnly}
                defaultModel={defaultModel}
                onStartEdit={startEdit}
                moreRows={moreRows}
                onOpenRow={(id) => setOpenSection(id as HarnessSettingsSection)}
              />
            </div>
          )}

          {activeTab !== "harness" && (
            <div className="min-w-0 pt-5 sm:pt-6">
              {activeTab === "preview" && (
                <HarnessPreview
                  systemPrompt={draft.fields.system_prompt}
                  parentHarnessId={draft.fields.parent_harness_id || undefined}
                  capabilities={draft.capabilities.map((cap) => ({
                    ref: cap.ref,
                    config: cap.config,
                  }))}
                  initialFiles={draft.files}
                />
              )}
              {activeTab === "integrate" && (
                <IntegrationGuide kind="harness" id={harness.id} name={displayName} />
              )}
              {activeTab === "stats" && (
                <ResourceStatsPanel stats={stats} isLoading={statsLoading} error={statsError} />
              )}
            </div>
          )}
        </div>

        <PageFooter>
          <BackLink href="/harnesses">Back to Harnesses</BackLink>
        </PageFooter>
      </PageContainer>

      <HarnessSettingsSheet
        section={openSection}
        onOpenChange={(open) => {
          if (!open) setOpenSection(null);
        }}
        harness={harness}
        draft={draft}
        editing={editing}
        readOnly={readOnly}
        onDraftChange={onDraftChange}
        onStartEdit={startEdit}
      />

      <Dialog
        open={confirmAction !== null}
        onOpenChange={(open) => {
          if (!open) setConfirmAction(null);
        }}
      >
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {confirmAction === "delete" ? "Delete harness" : "Archive harness"}
            </DialogTitle>
            <DialogDescription>
              {confirmAction === "delete"
                ? `Permanently delete the archived harness “${displayName}”? Existing references will render as deleted tombstones.`
                : `Archive “${displayName}”? It stays visible when archived items are shown, becomes read-only, and stops being assignable.`}
            </DialogDescription>
            {confirmError && confirmAction && (
              <EntityDeleteErrorNotice
                entityKind="harness"
                action={confirmAction}
                message={confirmError.message}
                className="mt-4"
              />
            )}
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setConfirmAction(null)}>
              Cancel
            </Button>
            <Button
              variant={confirmAction === "delete" ? "destructive" : "default"}
              onClick={handleConfirm}
              disabled={confirmPending}
            >
              {confirmAction === "delete"
                ? destroyHarness.isPending
                  ? "Deleting..."
                  : "Delete harness"
                : deleteHarness.isPending
                  ? "Archiving..."
                  : "Archive harness"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
