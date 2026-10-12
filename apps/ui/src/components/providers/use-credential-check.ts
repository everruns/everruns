"use client";

// Debounced, button-free credential verification.
//
// The provider forms probe a candidate credential against the provider while
// the user is still in the form (`POST /v1/providers/check-credentials`, which
// stores nothing). Verification that needs a click is verification most people
// skip, so it runs on input instead: a bad key is caught where it was pasted
// rather than at the first agent run.
//
// Only `rejected` proves a credential is bad. `unsupported` (driver declares no
// check) and `unreachable` (provider outage, offline or air-gapped install) say
// nothing about the credential and collapse into `inconclusive`, which must
// never block saving.

import { useCallback, useEffect, useRef, useState } from "react";
import { checkProviderCredentials } from "@/lib/api/providers";
import type { DriverId } from "@/lib/api/types";

export type CredentialCheckState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "valid"; models: number }
  | { status: "rejected"; message: string }
  | { status: "inconclusive" };

/** Typing pause before a probe fires. Long enough that a pasted key checks once. */
const DEBOUNCE_MS = 600;

export function useCredentialCheck({
  providerType,
  credentials,
  baseUrl,
  enabled = true,
}: {
  providerType: DriverId | undefined;
  /** Credential fields keyed by the driver's schema field names. */
  credentials: Record<string, string>;
  baseUrl?: string;
  enabled?: boolean;
}): { state: CredentialCheckState; recheck: () => void } {
  const [state, setState] = useState<CredentialCheckState>({ status: "idle" });
  // Bumped by `recheck` to re-run the effect on unchanged input.
  const [nonce, setNonce] = useState(0);
  // Monotonic request id: a response whose id is stale (input changed, or the
  // form cleared) is dropped rather than overwriting a newer verdict.
  const requestRef = useRef(0);

  // Serialized so the effect keys off the *values*, not the object identity a
  // caller re-creates on every render.
  const payload = JSON.stringify(
    Object.fromEntries(Object.entries(credentials).filter(([, v]) => v.trim() !== "")),
  );

  useEffect(() => {
    const filled = JSON.parse(payload) as Record<string, string>;
    if (!enabled || !providerType || Object.keys(filled).length === 0) {
      requestRef.current += 1;
      setState({ status: "idle" });
      return;
    }

    setState({ status: "checking" });
    requestRef.current += 1;
    const request = requestRef.current;

    const timer = setTimeout(() => {
      void (async () => {
        try {
          const result = await checkProviderCredentials({
            provider_type: providerType,
            credentials: filled,
            base_url: baseUrl || undefined,
          });
          if (requestRef.current !== request) return;
          if (result.status === "valid") {
            setState({ status: "valid", models: result.models });
          } else if (result.status === "rejected") {
            setState({ status: "rejected", message: result.message });
          } else {
            setState({ status: "inconclusive" });
          }
        } catch {
          // The probe itself failed (offline, 400 on a half-filled multi-field
          // schema). That is not evidence about the credential.
          if (requestRef.current !== request) return;
          setState({ status: "inconclusive" });
        }
      })();
    }, DEBOUNCE_MS);

    return () => clearTimeout(timer);
  }, [payload, providerType, baseUrl, enabled, nonce]);

  const recheck = useCallback(() => setNonce((n) => n + 1), []);

  return { state, recheck };
}
