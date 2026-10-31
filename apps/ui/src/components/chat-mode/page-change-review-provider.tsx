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
import { formResource, resourceKey, resourceWrite } from "./page-resource-history";
import { SavedChangesBar, UndoneBar } from "./saved-changes-bar";

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

/** Floats over the bottom of the page; renders nothing until a save produces a receipt. */
export function SavedPageChanges() {
  const ui = useUiTranslations();
  const controller = useContext(PageChangeReviewContext);
  const [history, setHistory] = useState<PageResource | null>(null);
  useSyncExternalStore(
    controller?.subscribe ?? noSubscribe,
    controller?.snapshot ?? noSnapshot,
    noSnapshot,
  );
  const receipts = controller?.receipts() ?? [],
    errors = controller?.errors() ?? [];
  if (!controller) return null;
  const undone = controller.undoneNotices();
  if (!receipts.length && !errors.length && !undone.length && !history) return null;
  return (
    <div
      aria-label={ui("Saved page changes")}
      className="pointer-events-none absolute inset-x-0 bottom-4 z-30 flex flex-col items-center gap-2 px-4"
    >
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
        <div
          key={`error:${resource.kind}:${resource.id}`}
          role="alert"
          className="pointer-events-auto flex max-w-full items-center gap-3 border bg-card px-3 py-1.5 text-sm shadow-lg"
        >
          <span className="truncate text-destructive">{error}</span>
          <button
            type="button"
            className="whitespace-nowrap underline underline-offset-2"
            onClick={() => void controller.retry(resource)}
          >
            {ui("Reload history")}
          </button>
        </div>
      ))}
      {receipts.map((receipt) => (
        <SavedChangesBar
          key={`${receipt.resource.kind}:${receipt.resource.id}`}
          receipt={receipt}
          dirty={controller.hasDirtyDraft(receipt.resource)}
          fieldLabels={
            receipt.members
              ? Object.fromEntries(receipt.members.map((item) => [item.id, item.label]))
              : undefined
          }
          onHistory={setHistory}
          onDismiss={() => controller.dismiss(receipt.resource)}
          onUndo={(field) => void controller.undo(receipt.resource, field)}
        />
      ))}
      {undone.map(({ resource, count }) => (
        <UndoneBar
          key={`undone:${resourceKey(resource)}`}
          count={count}
          resource={resource}
          onHistory={setHistory}
          onDone={() => controller.clearUndone(resource)}
        />
      ))}
    </div>
  );
}
