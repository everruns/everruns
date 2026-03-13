"use client";

import type { ReactNode } from "react";
import { usePolicies } from "@/hooks/use-policies";

interface PolicyGateProps {
  /** Resource name (e.g. "harnesses", "agents") */
  resource: string;
  /** Policy ID to check (e.g. "harness.manage") */
  policy: string;
  /** Content shown when policy is not satisfied */
  fallback?: ReactNode;
  children: ReactNode;
}

/**
 * Renders children only if the caller satisfies the given policy.
 * Shows fallback (or nothing) otherwise.
 */
export function PolicyGate({ resource, policy, fallback = null, children }: PolicyGateProps) {
  const { can, isLoading } = usePolicies(resource);

  if (isLoading) return fallback;
  if (!can(policy)) return fallback;

  return <>{children}</>;
}
