// Change history, restore and manager context API.
//
// Routes: GET /v1/history/{ref}, GET /v1/history/{ref}/revisions/{revision},
// GET /v1/history/{ref}/diff, POST /v1/history/{ref}/restore, and
// GET/PUT /v1/context/{ref}.
//
// Decision: a change reason is invocation metadata, not a body field. REST
// routes read it from the `Everruns-Change-Reason` header (UTF-8
// percent-encoded), so every mutating call takes the same optional `reason`
// and turns it into that header here. A blank reason sends no header: people
// may omit it, and the field never blocks a save.
// See knowledge/execution/change-reasons-and-manager-context.md.

import { api, type RequestOptions } from "./client";
import type {
  EntityChange,
  EntityRevision,
  FieldDiff,
  ManagerContext,
  RestoreResult,
} from "./types";

export const CHANGE_REASON_HEADER = "Everruns-Change-Reason";

/** Longest reason the server accepts, in characters. */
export const MAX_CHANGE_REASON_CHARS = 1000;

/** Error code the server returns when manager notes changed since they were read. */
export const MANAGER_CONTEXT_CHANGED = "manager_context_changed";

/** Request options carrying a change reason, or `undefined` when there is none. */
export function changeReasonOptions(reason?: string | null): RequestOptions | undefined {
  const trimmed = reason?.trim();
  if (!trimmed) return undefined;
  return { headers: { [CHANGE_REASON_HEADER]: encodeURIComponent(trimmed) } };
}

function refPath(entityRef: string): string {
  return encodeURIComponent(entityRef);
}

export interface EntityHistoryPage {
  entries: EntityChange[];
  /** Cursor for the next (older) page, absent at the end. */
  before?: string;
}

export const HISTORY_PAGE_SIZE = 50;

export async function listEntityHistory(
  entityRef: string,
  before?: string,
): Promise<EntityHistoryPage> {
  const params = new URLSearchParams({ limit: String(HISTORY_PAGE_SIZE) });
  if (before) params.set("before", before);
  const response = await api.get<EntityChange[]>(
    `/v1/history/${refPath(entityRef)}?${params.toString()}`,
  );
  const entries = response.data;
  return {
    entries,
    before:
      entries.length === HISTORY_PAGE_SIZE ? entries[entries.length - 1]?.created_at : undefined,
  };
}

export async function getEntityRevision(
  entityRef: string,
  revision: number,
): Promise<EntityRevision> {
  const response = await api.get<EntityRevision>(
    `/v1/history/${refPath(entityRef)}/revisions/${revision}`,
  );
  return response.data;
}

/** Fields that differ between `from` and `to`; `to` omitted means the current state. */
export async function diffEntityRevisions(
  entityRef: string,
  from: number,
  to?: number,
): Promise<FieldDiff[]> {
  const params = new URLSearchParams({ from: String(from) });
  if (to !== undefined) params.set("to", String(to));
  const response = await api.get<FieldDiff[]>(
    `/v1/history/${refPath(entityRef)}/diff?${params.toString()}`,
  );
  return response.data;
}

/** Bring a revision back as a new change. A restore always carries a reason. */
export async function restoreEntityRevision(
  entityRef: string,
  revision: number,
  reason: string,
): Promise<RestoreResult> {
  const response = await api.post<RestoreResult>(
    `/v1/history/${refPath(entityRef)}/restore`,
    { revision },
    changeReasonOptions(reason),
  );
  return response.data;
}

export async function getManagerContext(entityRef: string): Promise<ManagerContext> {
  const response = await api.get<ManagerContext>(`/v1/context/${refPath(entityRef)}`);
  return response.data;
}

/**
 * Replace the notes. `expectedRevision` is the revision the edit was based on;
 * the server refuses a stale one with `manager_context_changed`.
 */
export async function setManagerContext(
  entityRef: string,
  content: string,
  expectedRevision: number,
  reason?: string,
): Promise<ManagerContext> {
  const response = await api.put<ManagerContext>(
    `/v1/context/${refPath(entityRef)}`,
    { content, expected_revision: expectedRevision },
    changeReasonOptions(reason),
  );
  return response.data;
}

/** Secret fields in a snapshot diff are `$secrets.<name>`. */
export const SECRET_FIELD_PREFIX = "$secrets.";

export function isSecretField(field: string): boolean {
  return field === "$secrets" || field.startsWith(SECRET_FIELD_PREFIX);
}

export function secretFieldName(field: string): string {
  return field.startsWith(SECRET_FIELD_PREFIX) ? field.slice(SECRET_FIELD_PREFIX.length) : field;
}
