import { api } from "./client";
import { changeReasonOptions } from "./change-history";
import type { ListResponse } from "./types";

function withArchivedQuery(basePath: string, includeArchived: boolean): string {
  if (!includeArchived) {
    return basePath;
  }

  const separator = basePath.includes("?") ? "&" : "?";
  return `${basePath}${separator}include_archived=true`;
}

export interface CrudApi<TItem, TCreate, TUpdate> {
  create(request: TCreate): Promise<TItem>;
  list(includeArchived?: boolean): Promise<TItem[]>;
  get(id: string): Promise<TItem>;
  /** `reason` is the optional change reason recorded in the entity's history. */
  update(id: string, request: TUpdate, reason?: string): Promise<TItem>;
  delete(id: string, reason?: string): Promise<void>;
  destroy(id: string, reason?: string): Promise<void>;
}

export function createCrudApi<TItem, TCreate, TUpdate>(
  basePath: string,
): CrudApi<TItem, TCreate, TUpdate> {
  return {
    async create(request: TCreate): Promise<TItem> {
      const response = await api.post<TItem>(basePath, request);
      return response.data;
    },

    async list(includeArchived = false): Promise<TItem[]> {
      const response = await api.get<ListResponse<TItem>>(
        withArchivedQuery(basePath, includeArchived),
      );
      return response.data.data;
    },

    async get(id: string): Promise<TItem> {
      const response = await api.get<TItem>(`${basePath}/${id}`);
      return response.data;
    },

    async update(id: string, request: TUpdate, reason?: string): Promise<TItem> {
      const response = await api.patch<TItem>(
        `${basePath}/${id}`,
        request,
        changeReasonOptions(reason),
      );
      return response.data;
    },

    async delete(id: string, reason?: string): Promise<void> {
      await api.delete(`${basePath}/${id}`, changeReasonOptions(reason));
    },

    async destroy(id: string, reason?: string): Promise<void> {
      await api.post(`${basePath}/${id}/delete`, undefined, changeReasonOptions(reason));
    },
  };
}
