"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { HistorySheet } from "@/components/entity-actions/history-sheet";
import type { PageResource } from "./page-resource-history";
import { useQueryClient } from "@tanstack/react-query";
import { useOrg } from "@/providers/org-provider";
import { getActiveOrgId } from "@/lib/api/active-org";
import {
  listEntityHistory,
  diffEntityRevisions,
  restoreEntityRevision,
} from "@/lib/api/change-history";
import { undoModelSelection } from "@/lib/api/model-selection";
import { observeModelSelection, observeResourceWrites } from "@/lib/api/write-observer";
import { useChatMode } from "./chat-mode-context";
import { PageChangeReviewController } from "./page-change-review-controller";
import { formResource, resourceWrite } from "./page-resource-history";
import { PageChangeDiff } from "./page-change-diff";

export const PageChangeReviewContext = createContext<PageChangeReviewController | null>(null);

export function PageChangeReviewProvider({ children }: { children: ReactNode }) {
  const { currentOrg } = useOrg();
  return currentOrg ? (
    <Enabled key={currentOrg.public_id} org={currentOrg.public_id}>
      {children}
    </Enabled>
  ) : (
    children
  );
}
function Enabled({ org, children }: { org: string; children: ReactNode }) {
  const client = useQueryClient(),
    mode = useChatMode();
  const controller = useMemo(
    () =>
      new PageChangeReviewController(org, {
        currentOrg: getActiveOrgId,
        undoSelection: undoModelSelection,
        history: async (resource) =>
          (
            await listEntityHistory(
              resource.id,
              undefined,
              resource.kind,
              AbortSignal.timeout(12_000),
            )
          ).entries,
        diff: (resource, from, to) =>
          diffEntityRevisions(resource.id, from, to, resource.kind, AbortSignal.timeout(12_000)),
        restore: async (resource, from, revision, fields) => {
          const result = await restoreEntityRevision(
            resource.id,
            from,
            "Undo reviewed page changes",
            { kind: resource.kind, expected_revision: revision, fields },
          );
          return result.revision ?? undefined;
        },
        refresh: async () => {
          await client.invalidateQueries({ type: "active" });
        },
      }),
    [org, client],
  );
  // Explicit registered identities take priority over a known detail route.
  const activeResource =
    formResource(mode?.page.entity_type ?? "", mode?.page.entity_id, mode?.page.form_name) ??
    resourceWrite(
      "PATCH",
      `/v1${(mode?.page.route ?? "").replace(/^\/models\/providers\//, "/providers/")}`,
    );
  useLayoutEffect(() => {
    controller.watch(activeResource, mode?.threadId);
  }, [controller, activeResource, mode?.threadId]);
  useEffect(() => {
    controller.activate();
    const stop = observeResourceWrites(controller.beforeWrite);
    const stopSelection = observeModelSelection(controller.recordSelection);
    return () => {
      stop();
      stopSelection();
      controller.dispose();
    };
  }, [controller]);
  useEffect(() => {
    if (!mode?.open) return;
    void controller.poll();
    const timer = setInterval(() => void controller.poll(), 1500);
    return () => clearInterval(timer);
  }, [controller, mode?.open, mode?.page.route]);
  return (
    <PageChangeReviewContext.Provider value={controller}>
      {children}
    </PageChangeReviewContext.Provider>
  );
}

const noSubscribe = () => () => {};
const noSnapshot = () => 0;
export function usePageSavedChangeGuard(options: {
  id: string;
  entityType: string;
  entityId?: string;
  formId?: string;
  dirty: boolean;
  baseline: string;
  enabled: boolean;
}) {
  const controller = useContext(PageChangeReviewContext);
  useSyncExternalStore(
    controller?.subscribe ?? noSubscribe,
    controller?.snapshot ?? noSnapshot,
    noSnapshot,
  );
  const resource = formResource(
    options.entityType,
    options.entityId ??
      (["organization_network", "web_search_defaults", "model_defaults"].includes(
        options.entityType,
      )
        ? controller?.organizationId
        : undefined),
    options.formId,
  );
  const key = resource ? `${resource.kind}:${resource.id}` : "";
  useLayoutEffect(() => {
    if (controller && resource && options.enabled)
      return controller.connect({
        id: options.id,
        resource,
        dirty: options.dirty,
        baseline: options.baseline,
      });
    // Dirty/baseline updates below must not reconnect and reset the saved revision.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [controller, key, options.id, options.enabled]);
  useLayoutEffect(() => {
    if (controller && resource && options.enabled)
      controller.update({
        id: options.id,
        resource,
        dirty: options.dirty,
        baseline: options.baseline,
      });
  });
  return controller && resource && options.enabled
    ? controller.guard(options.id)
    : { loading: false, conflict: false };
}

export function SavedPageChanges() {
  const ui = useUiTranslations();
  const controller = useContext(PageChangeReviewContext);
  const [history, setHistory] = useState<PageResource | null>(null);
  useSyncExternalStore(
    controller?.subscribe ?? noSubscribe,
    controller?.snapshot ?? noSnapshot,
    noSnapshot,
  );
  if (!controller) return null;
  const receipts = controller.receipts(),
    errors = controller.errors();
  const active = controller.activeResource;
  if (!receipts.length && !errors.length && !active) return null;
  return (
    <aside
      aria-label={ui("Saved page changes")}
      className="shrink-0 max-h-[45vh] overflow-auto border-t border-accent bg-card px-4 py-3"
    >
      {active && (
        <button type="button" className="text-sm underline" onClick={() => setHistory(active)}>
          {ui("History ·")} {active.label}
        </button>
      )}
      {history && (
        <HistorySheet
          key={`${history.kind}:${history.id}`}
          entityRef={history.id}
          kind={history.kind}
          entityName={history.label}
          canRestore={true}
          open={true}
          onOpenChange={(open) => {
            if (!open) setHistory(null);
          }}
        />
      )}
      {errors.map(({ resource, error }) => (
        <div key={`error:${resource.kind}:${resource.id}`} role="alert" className="mb-2 text-sm">
          <p>{error}</p>
          <button
            type="button"
            className="underline"
            onClick={() => void controller.retry(resource)}
          >
            {ui("Reload history")}
          </button>
        </div>
      ))}
      {receipts.map((receipt) => {
        const dirty = controller.hasDirtyDraft(receipt.resource);
        return (
          <section key={`${receipt.resource.kind}:${receipt.resource.id}`} className="py-2">
            <div className="flex flex-wrap items-center gap-3 text-sm">
              <strong className="flex-1">
                {ui("Saved changes ·")} {receipt.resource.label}
              </strong>
              <button
                type="button"
                className="underline disabled:opacity-50"
                disabled={receipt.pending || receipt.stale || dirty}
                onClick={() => void controller.undo(receipt.resource)}
              >
                {receipt.pending ? ui("Undoing…") : ui("Undo all")}
              </button>
              {receipt.resource.kind !== "model_selection" && (
                <button
                  type="button"
                  className="underline"
                  onClick={() => setHistory(receipt.resource)}
                >
                  {ui("History")}
                </button>
              )}
              <button
                type="button"
                className="underline"
                onClick={() => controller.dismiss(receipt.resource)}
              >
                {ui("Dismiss")}
              </button>
            </div>
            {receipt.stale && (
              <p className="text-sm text-muted-foreground">
                {ui("This resource changed again. Review its history before undoing.")}
              </p>
            )}
            {dirty && (
              <p className="text-sm text-muted-foreground">
                {ui("Your draft is kept. Save or discard it before undoing saved changes.")}
              </p>
            )}
            <PageChangeDiff
              fields={receipt.fields}
              fieldLabels={
                receipt.members
                  ? Object.fromEntries(receipt.members.map((item) => [item.id, item.label]))
                  : undefined
              }
              pending={receipt.pending}
              canUndo={!receipt.stale && !dirty}
              undo={(field) => void controller.undo(receipt.resource, field)}
            />
            {receipt.error && (
              <p role="alert" className="text-sm text-destructive">
                {receipt.error}
              </p>
            )}
          </section>
        );
      })}
    </aside>
  );
}
