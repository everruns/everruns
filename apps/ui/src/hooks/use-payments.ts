"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createPaymentAccount,
  createPaymentPolicy,
  disablePaymentAccount,
  disablePaymentPolicy,
  listPaymentAccounts,
  listPaymentAttempts,
  listPaymentPolicies,
  updatePaymentAccount,
  updatePaymentPolicy,
} from "@/lib/api/payments";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import type {
  CreatePaymentAccountRequest,
  CreatePaymentPolicyRequest,
  UpdatePaymentAccountRequest,
  UpdatePaymentPolicyRequest,
} from "@/lib/api/types";

export function usePaymentAccounts(params: { owner_type?: string; owner_id?: string } = {}) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;
  const query = useQuery({
    queryKey: [...queryKeys.payments.accounts(params), org],
    queryFn: () => listPaymentAccounts(params),
    enabled: !!org,
  });
  return { ...query, isLoading: orgLoading || query.isLoading };
}

export function useCreatePaymentAccount() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: CreatePaymentAccountRequest) => createPaymentAccount(input),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.payments.all }),
  });
}

export function useUpdatePaymentAccount() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      paymentAccountId,
      input,
    }: {
      paymentAccountId: string;
      input: UpdatePaymentAccountRequest;
    }) => updatePaymentAccount(paymentAccountId, input),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.payments.all }),
  });
}

export function useDisablePaymentAccount() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (paymentAccountId: string) => disablePaymentAccount(paymentAccountId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.payments.all }),
  });
}

export function usePaymentPolicies(
  params: { payment_account_id?: string; subject_type?: string; subject_id?: string } = {},
) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;
  const query = useQuery({
    queryKey: [...queryKeys.payments.policies(params), org],
    queryFn: () => listPaymentPolicies(params),
    enabled: !!org,
  });
  return { ...query, isLoading: orgLoading || query.isLoading };
}

export function useCreatePaymentPolicy() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: CreatePaymentPolicyRequest) => createPaymentPolicy(input),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.payments.all }),
  });
}

export function useUpdatePaymentPolicy() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      paymentPolicyId,
      input,
    }: {
      paymentPolicyId: string;
      input: UpdatePaymentPolicyRequest;
    }) => updatePaymentPolicy(paymentPolicyId, input),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.payments.all }),
  });
}

export function useDisablePaymentPolicy() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (paymentPolicyId: string) => disablePaymentPolicy(paymentPolicyId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: queryKeys.payments.all }),
  });
}

export function usePaymentAttempts(params: { session_id?: string; limit?: number } = {}) {
  const { currentOrg, isLoading: orgLoading } = useOrg();
  const org = currentOrg?.public_id;
  const query = useQuery({
    queryKey: [...queryKeys.payments.attempts(params), org],
    queryFn: () => listPaymentAttempts(params),
    enabled: !!org,
  });
  return { ...query, isLoading: orgLoading || query.isLoading };
}
