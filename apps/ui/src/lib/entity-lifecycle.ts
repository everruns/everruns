import type { VariantProps } from "class-variance-authority";
import { badgeVariants } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

export type EntityLifecycleStatus =
  | "active"
  | "archived"
  | "deleted"
  | "disabled"
  | "draft"
  | "published";

export type EntityKind = "Agent" | "Harness" | "Skill" | "MCP Server" | "App";

export function isArchivedStatus(
  status: EntityLifecycleStatus | string | null | undefined,
): boolean {
  return status === "archived";
}

export function isDeletedStatus(
  status: EntityLifecycleStatus | string | null | undefined,
): boolean {
  return status === "deleted";
}

export function isReadOnlyStatus(
  status: EntityLifecycleStatus | string | null | undefined,
): boolean {
  return isArchivedStatus(status) || isDeletedStatus(status);
}

export function getEntityStatusBadgeVariant(
  status: EntityLifecycleStatus | string,
): VariantProps<typeof badgeVariants>["variant"] {
  switch (status) {
    case "active":
    case "published":
      return "default";
    case "deleted":
      return "destructive";
    case "archived":
      return "secondary";
    case "disabled":
    case "draft":
    default:
      return "outline";
  }
}

export function getEntityNameClassName(status: EntityLifecycleStatus | string | null | undefined) {
  return cn(isArchivedStatus(status) && "text-muted-foreground line-through");
}

export function getDeletedEntityLabel(kind: EntityKind): string {
  return `<Deleted ${kind}>`;
}

export function getEntityReferenceLabel(params: {
  kind: EntityKind;
  name?: string | null;
  status?: EntityLifecycleStatus | string | null;
}): string {
  const { kind, name, status } = params;
  if (isDeletedStatus(status) || !name) {
    return getDeletedEntityLabel(kind);
  }
  return name;
}

export function getEntityReferenceClassName(status?: EntityLifecycleStatus | string | null) {
  return cn(
    isArchivedStatus(status) && "text-muted-foreground line-through",
    isDeletedStatus(status) && "text-destructive",
  );
}
