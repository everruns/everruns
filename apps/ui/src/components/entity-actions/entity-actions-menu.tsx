"use client";

// EntityActionsMenu: the one header overflow menu every entity detail page
// carries (knowledge/ui/entity-actions-menu.md).
//
// Decisions:
// - Fixed groups in a fixed order: the kind's own entity actions, then Record
//   (History, Manager notes), then Lifecycle (Archive, Delete; Delete
//   destructive and last). A group with no visible items is omitted along with
//   its divider, so a person who learns the menu on one page finds every item
//   in the same place on the next.
// - Items open, they do not navigate: History and Manager notes open side
//   sheets rendered here; Archive and Delete call back into the page, which
//   owns its confirm dialog (each kind words its own consequences).
// - The open sheet is part of the address (`?sheet=history`, `?sheet=notes`),
//   so a refresh or a shared link reopens it.
// - Hidden, not disabled, when the viewer can never use an item: Manager notes
//   appear only for viewers who manage the entity. Disabled means "not right
//   now" and carries its reason as a tooltip.
// - No badge on the trigger. Where note state matters, the page shows one
//   muted line in edit mode (see `ManagerNotesHint`).

import type { ComponentProps, ReactNode } from "react";
import { useCallback } from "react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { Archive, History, MoreHorizontal, NotebookPen, Trash2 } from "lucide-react";
import { buttonVariants } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuPositioner,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import dynamic from "next/dynamic";
import { entityKindLabel, type EntityKind } from "@/components/entity-actions/entity-kinds";

export { entityKindLabel, type EntityKind };

// The sheets load on first open: every entity page carries the menu, few
// visits open History or Manager notes, and the notes sheet pulls in the
// markdown renderer.
const HistorySheet = dynamic(() =>
  import("@/components/entity-actions/history-sheet").then((m) => m.HistorySheet),
);
const ManagerNotesSheet = dynamic(() =>
  import("@/components/entity-actions/manager-notes-sheet").then((m) => m.ManagerNotesSheet),
);

export type EntitySheet = "history" | "notes";

export const ENTITY_SHEET_PARAM = "sheet";

function parseSheet(value: string | null): EntitySheet | null {
  return value === "history" || value === "notes" ? value : null;
}

/** The query string with `sheet` set (or removed when `sheet` is null). */
export function entitySheetSearch(
  search: { toString(): string },
  sheet: EntitySheet | null,
): string {
  const params = new URLSearchParams(search.toString());
  if (sheet) params.set(ENTITY_SHEET_PARAM, sheet);
  else params.delete(ENTITY_SHEET_PARAM);
  const query = params.toString();
  return query ? `?${query}` : "";
}

/** The open record sheet, read from and written to the address. */
export function useEntitySheet(): [EntitySheet | null, (sheet: EntitySheet | null) => void] {
  const router = useRouter();
  const pathname = usePathname();
  const searchParams = useSearchParams();
  const sheet = parseSheet(searchParams.get(ENTITY_SHEET_PARAM));
  const setSheet = useCallback(
    (next: EntitySheet | null) => {
      router.replace(`${pathname}${entitySheetSearch(searchParams, next)}`, { scroll: false });
    },
    [pathname, router, searchParams],
  );
  return [sheet, setSheet];
}

/** One kind-specific item in the entity actions group. */
export interface EntityAction {
  id: string;
  label: ReactNode;
  icon?: ReactNode;
  onSelect?: () => void;
  /** Rare: an item that leaves the page. Prefer a link on the page itself. */
  href?: ComponentProps<typeof Link>["href"];
  disabled?: boolean;
  /** Why the item is disabled right now, shown as its tooltip. */
  disabledReason?: string;
}

export interface LifecycleAction {
  onSelect: () => void;
  /** Defaults to "Archive {kind}" / "Delete {kind}". */
  label?: ReactNode;
  disabled?: boolean;
  disabledReason?: string;
}

export interface EntityPermissions {
  /** Can read the entity's history. Defaults to true: history is visible where the entity is. */
  history?: boolean;
  /** Manages the entity: reads and writes manager notes and may restore a revision. */
  manage: boolean;
}

export interface EntityActionsMenuProps {
  entityRef: string;
  kind: EntityKind;
  /** Display name, used in the trigger's accessible label and sheet titles. */
  entityName: string;
  permissions: EntityPermissions;
  /** The kind's own items, ordered by frequency. */
  actions?: EntityAction[];
  /** Lifecycle items; omit one the viewer cannot use (hidden, not disabled). */
  archive?: LifecycleAction;
  delete?: LifecycleAction;
}

type MenuItemModel =
  | { kind: "action"; id: string; action: EntityAction }
  | { kind: "sheet"; id: EntitySheet; label: string; icon: ReactNode }
  | {
      kind: "lifecycle";
      id: "archive" | "delete";
      label: ReactNode;
      icon: ReactNode;
      action: LifecycleAction;
      destructive: boolean;
    };

export interface MenuGroupModel {
  id: "entity" | "record" | "lifecycle";
  items: MenuItemModel[];
}

/** Groups in their fixed order, empty ones dropped. Exported for tests. */
export function buildEntityMenuGroups({
  kind,
  permissions,
  actions = [],
  archive,
  delete: del,
}: Pick<
  EntityActionsMenuProps,
  "kind" | "permissions" | "actions" | "archive" | "delete"
>): MenuGroupModel[] {
  const label = entityKindLabel(kind);
  const record: MenuItemModel[] = [];
  if (permissions.history ?? true) {
    record.push({ kind: "sheet", id: "history", label: "History", icon: <History /> });
  }
  if (permissions.manage) {
    record.push({ kind: "sheet", id: "notes", label: "Manager notes", icon: <NotebookPen /> });
  }
  const lifecycle: MenuItemModel[] = [];
  if (archive) {
    lifecycle.push({
      kind: "lifecycle",
      id: "archive",
      label: archive.label ?? `Archive ${label}`,
      icon: <Archive />,
      action: archive,
      destructive: false,
    });
  }
  if (del) {
    lifecycle.push({
      kind: "lifecycle",
      id: "delete",
      label: del.label ?? `Delete ${label}`,
      icon: <Trash2 />,
      action: del,
      destructive: true,
    });
  }
  const groups: MenuGroupModel[] = [
    {
      id: "entity",
      items: actions.map((action) => ({ kind: "action" as const, id: action.id, action })),
    },
    { id: "record", items: record },
    { id: "lifecycle", items: lifecycle },
  ];
  return groups.filter((group) => group.items.length > 0);
}

export function EntityActionsMenu(props: EntityActionsMenuProps) {
  const { entityRef, kind, entityName, permissions } = props;
  const [sheet, setSheet] = useEntitySheet();
  const groups = buildEntityMenuGroups(props);

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          className={buttonVariants({ variant: "outline", size: "icon" })}
          aria-label={`More actions for ${entityName}`}
          data-slot="entity-actions-trigger"
        >
          <MoreHorizontal className="size-4" />
        </DropdownMenuTrigger>
        <DropdownMenuPositioner align="end">
          <DropdownMenuContent className="min-w-48">
            {groups.map((group, index) => (
              <DropdownMenuGroup key={group.id} data-group={group.id}>
                {index > 0 && <DropdownMenuSeparator />}
                {group.items.map((item) => (
                  <EntityMenuItem key={item.id} item={item} onOpenSheet={setSheet} />
                ))}
              </DropdownMenuGroup>
            ))}
          </DropdownMenuContent>
        </DropdownMenuPositioner>
      </DropdownMenu>
      <EntityRecordSheets
        entityRef={entityRef}
        kind={kind}
        entityName={entityName}
        permissions={permissions}
        sheet={sheet}
        onSheetChange={setSheet}
      />
    </>
  );
}

function EntityMenuItem({
  item,
  onOpenSheet,
}: {
  item: MenuItemModel;
  onOpenSheet: (sheet: EntitySheet) => void;
}) {
  if (item.kind === "sheet") {
    return (
      <DropdownMenuItem onClick={() => onOpenSheet(item.id)}>
        {item.icon}
        {item.label}
      </DropdownMenuItem>
    );
  }
  if (item.kind === "lifecycle") {
    return (
      <DropdownMenuItem
        onClick={item.action.onSelect}
        disabled={item.action.disabled}
        title={item.action.disabled ? item.action.disabledReason : undefined}
        variant={item.destructive ? "destructive" : "default"}
      >
        {item.icon}
        {item.label}
      </DropdownMenuItem>
    );
  }
  const { action } = item;
  const title = action.disabled ? action.disabledReason : undefined;
  if (action.href) {
    return (
      <DropdownMenuItem
        render={<Link href={action.href} />}
        disabled={action.disabled}
        title={title}
      >
        {action.icon}
        {action.label}
      </DropdownMenuItem>
    );
  }
  return (
    <DropdownMenuItem onClick={action.onSelect} disabled={action.disabled} title={title}>
      {action.icon}
      {action.label}
    </DropdownMenuItem>
  );
}

interface EntityRecordSheetsProps {
  entityRef: string;
  kind: EntityKind;
  entityName: string;
  permissions: EntityPermissions;
  /** Controlled sheet; defaults to the address (`?sheet=`). */
  sheet?: EntitySheet | null;
  onSheetChange?: (sheet: EntitySheet | null) => void;
}

/**
 * The History and Manager notes sheets on their own, for a page state that
 * hides the menu trigger (the agent page in edit mode) but must still honor
 * `?sheet=` links such as the manager-notes hint.
 */
export function EntityRecordSheets(props: EntityRecordSheetsProps) {
  const [addressSheet, setAddressSheet] = useEntitySheet();
  const sheet = props.sheet !== undefined ? props.sheet : addressSheet;
  const setSheet = props.onSheetChange ?? setAddressSheet;
  const { entityRef, kind, entityName, permissions } = props;
  const close = (open: boolean) => {
    if (!open) setSheet(null);
  };
  return (
    <>
      {/* Mounted only while open, so a page pays no history or notes request
          until someone asks for one. */}
      {(permissions.history ?? true) && sheet === "history" && (
        <HistorySheet
          entityRef={entityRef}
          kind={kind}
          entityName={entityName}
          canRestore={permissions.manage}
          open
          onOpenChange={close}
        />
      )}
      {permissions.manage && sheet === "notes" && (
        <ManagerNotesSheet
          entityRef={entityRef}
          kind={kind}
          entityName={entityName}
          open
          onOpenChange={close}
        />
      )}
    </>
  );
}
