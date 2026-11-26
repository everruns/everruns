// Budget API client. The UI gates budget management with the `budget.view` /
// `budget.manage` policies.

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
  const response = await api.get<Budget[]>(`/v1/platform/budgets${qs ? `?${qs}` : ""}`);
  return response.data;
}

export async function createBudget(input: CreateBudgetRequest): Promise<Budget> {
  const response = await api.post<Budget>("/v1/platform/budgets", input);
  return response.data;
}

export async function updateBudget(budgetId: string, input: UpdateBudgetRequest): Promise<Budget> {
  const response = await api.patch<Budget>(`/v1/platform/budgets/${budgetId}`, input);
  return response.data;
}

export async function deleteBudget(budgetId: string): Promise<void> {
  await api.delete(`/v1/platform/budgets/${budgetId}`);
}
