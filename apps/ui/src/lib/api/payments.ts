import { api } from "./client";
import type {
  CreatePaymentAccountRequest,
  CreatePaymentPolicyRequest,
  PaymentAccount,
  PaymentAttempt,
  PaymentPolicy,
  UpdatePaymentAccountRequest,
  UpdatePaymentPolicyRequest,
} from "./types";

export async function listPaymentAccounts(params?: {
  owner_type?: string;
  owner_id?: string;
}): Promise<PaymentAccount[]> {
  const search = new URLSearchParams();
  if (params?.owner_type) search.set("owner_type", params.owner_type);
  if (params?.owner_id) search.set("owner_id", params.owner_id);
  const qs = search.toString();
  const response = await api.get<PaymentAccount[]>(`/v1/payments/accounts${qs ? `?${qs}` : ""}`);
  return response.data;
}

export async function createPaymentAccount(
  input: CreatePaymentAccountRequest,
): Promise<PaymentAccount> {
  const response = await api.post<PaymentAccount>("/v1/payments/accounts", input);
  return response.data;
}

export async function updatePaymentAccount(
  paymentAccountId: string,
  input: UpdatePaymentAccountRequest,
): Promise<PaymentAccount> {
  const response = await api.patch<PaymentAccount>(
    `/v1/payments/accounts/${paymentAccountId}`,
    input,
  );
  return response.data;
}

export async function disablePaymentAccount(paymentAccountId: string): Promise<void> {
  await api.delete(`/v1/payments/accounts/${paymentAccountId}`);
}

export async function listPaymentPolicies(params?: {
  payment_account_id?: string;
  subject_type?: string;
  subject_id?: string;
}): Promise<PaymentPolicy[]> {
  const search = new URLSearchParams();
  if (params?.payment_account_id) search.set("payment_account_id", params.payment_account_id);
  if (params?.subject_type) search.set("subject_type", params.subject_type);
  if (params?.subject_id) search.set("subject_id", params.subject_id);
  const qs = search.toString();
  const response = await api.get<PaymentPolicy[]>(`/v1/payments/policies${qs ? `?${qs}` : ""}`);
  return response.data;
}

export async function createPaymentPolicy(
  input: CreatePaymentPolicyRequest,
): Promise<PaymentPolicy> {
  const response = await api.post<PaymentPolicy>("/v1/payments/policies", input);
  return response.data;
}

export async function updatePaymentPolicy(
  paymentPolicyId: string,
  input: UpdatePaymentPolicyRequest,
): Promise<PaymentPolicy> {
  const response = await api.patch<PaymentPolicy>(
    `/v1/payments/policies/${paymentPolicyId}`,
    input,
  );
  return response.data;
}

export async function disablePaymentPolicy(paymentPolicyId: string): Promise<void> {
  await api.delete(`/v1/payments/policies/${paymentPolicyId}`);
}

export async function listPaymentAttempts(params?: {
  session_id?: string;
  limit?: number;
}): Promise<PaymentAttempt[]> {
  const search = new URLSearchParams();
  if (params?.session_id) search.set("session_id", params.session_id);
  if (params?.limit) search.set("limit", String(params.limit));
  const qs = search.toString();
  const response = await api.get<PaymentAttempt[]>(`/v1/payments/attempts${qs ? `?${qs}` : ""}`);
  return response.data;
}
