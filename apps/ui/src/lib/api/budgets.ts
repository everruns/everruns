// Budget API client. The UI gates budget management with `app_budgets`.

import { api } from "./client";
import type { Budget, CreateBudgetRequest, UpdateBudgetRequest } from "./types";

export async function listBudgets(params?: {
  subject_type?: string;
  subject_id?: string;
}): Promise<Budget[]> {
  const search = new URLSearchParams();
  if (params?.subject_type) search.set("subject_type", params.subject_type);
  if (params?.subject_id) search.set("subject_id", params.subject_id);
  const qs = search.toString();
  const response = await api.get<Budget[]>(`/v1/budgets${qs ? `?${qs}` : ""}`);
  return response.data;
}

export async function listAppBudgets(
  appId: string,
  options: { includeChannels?: boolean } = {},
): Promise<Budget[]> {
  const params = new URLSearchParams();
  if (options.includeChannels === false) params.set("include_channels", "false");
  const qs = params.toString();
  const response = await api.get<Budget[]>(`/v1/apps/${appId}/budgets${qs ? `?${qs}` : ""}`);
  return response.data;
}

export async function createBudget(input: CreateBudgetRequest): Promise<Budget> {
  const response = await api.post<Budget>("/v1/budgets", input);
  return response.data;
}

export async function updateBudget(budgetId: string, input: UpdateBudgetRequest): Promise<Budget> {
  const response = await api.patch<Budget>(`/v1/budgets/${budgetId}`, input);
  return response.data;
}

export async function deleteBudget(budgetId: string): Promise<void> {
  await api.delete(`/v1/budgets/${budgetId}`);
}
