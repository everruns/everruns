/**
 * OSS-owned zero-organization onboarding surface.
 *
 * Decision: SaaS-style wrappers compose this surface through extension points
 * instead of forking the whole page. The OSS component owns the layout, the
 * org-name form, current-org selection after create, and the `/orgs/{id}/setup`
 * redirect. Wrappers only supply policy/status and (optionally) a create
 * override:
 *  - `usePolicy` decides whether the form is shown, replaced by a blocked gate
 *    (e.g. "Verify your email"), or still loading.
 *  - `useCreateOrg` overrides the create mutation while keeping the OSS layout,
 *    current-org selection, and setup redirect.
 */
"use client";

import { useState, type ReactNode } from "react";
import { useRouter } from "next/navigation";
import { Building2, Loader2 } from "lucide-react";
import { useCreateOrganization } from "@/hooks/use-organizations";
import { useOrg } from "@/providers/org-provider";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

/** State returned by a zero-org policy hook. */
export type ZeroOrgPolicyState =
  | { status: "loading" }
  | { status: "ready" }
  | {
      /** Block org creation behind a wrapper gate (e.g. email verification). */
      status: "blocked";
      title: string;
      body?: ReactNode;
      actions?: ReactNode;
    };

export type UseZeroOrgPolicy = () => ZeroOrgPolicyState;

/** Default OSS policy: any authenticated user may create their first org. */
export const useReadyZeroOrgPolicy: UseZeroOrgPolicy = () => ({ status: "ready" });

export interface ZeroOrgOnboardingProps {
  /** Policy hook gating the form. Defaults to always-ready (OSS behavior). */
  usePolicy?: UseZeroOrgPolicy;
  /**
   * Create-organization mutation hook. Defaults to the OSS
   * `useCreateOrganization`. Overrides must expose the same
   * `{ mutateAsync, isPending, isError, error }` shape and resolve to
   * `{ id, name }`.
   */
  useCreateOrg?: typeof useCreateOrganization;
}

export function ZeroOrgOnboarding({
  usePolicy = useReadyZeroOrgPolicy,
  useCreateOrg = useCreateOrganization,
}: ZeroOrgOnboardingProps = {}) {
  const router = useRouter();
  const { setCurrentOrg } = useOrg();
  const createOrg = useCreateOrg();
  const policy = usePolicy();
  const [name, setName] = useState("");

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) return;
    try {
      const org = await createOrg.mutateAsync({ name: trimmed });
      setCurrentOrg({ public_id: org.id, name: org.name, role: "owner" });
      router.push(`/orgs/${org.id}/setup`);
    } catch {
      // Failure is surfaced via createOrg.isError / createOrg.error below;
      // swallow the rejection so it isn't an unhandled promise rejection.
    }
  };

  return (
    <div className="flex min-h-screen items-center justify-center bg-background bg-brand-dots p-4">
      <Card className="w-full max-w-md p-8">
        <div className="flex flex-col items-center gap-2 text-center">
          <div className="flex h-12 w-12 items-center justify-center bg-primary/10">
            <Building2 className="h-6 w-6 text-primary" />
          </div>
          <h1 className="text-xl font-semibold">Create your organization</h1>
          <p className="text-sm text-muted-foreground">
            Organizations own your agents, sessions, and settings. Create one to get started.
          </p>
        </div>

        <div className="mt-8">
          {policy.status === "loading" ? (
            <div className="flex justify-center py-6">
              <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
            </div>
          ) : policy.status === "blocked" ? (
            <div className="space-y-4 text-center" role="alert">
              <h2 className="text-base font-semibold">{policy.title}</h2>
              {policy.body && <div className="text-sm text-muted-foreground">{policy.body}</div>}
              {policy.actions && <div className="flex justify-center gap-2">{policy.actions}</div>}
            </div>
          ) : (
            <form onSubmit={handleSubmit} className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="onboarding-org-name">Organization name</Label>
                <Input
                  id="onboarding-org-name"
                  value={name}
                  onChange={(event) => setName(event.target.value)}
                  placeholder="Acme Inc."
                  required
                />
              </div>
              {createOrg.isError && (
                <p className="text-sm text-destructive">
                  Failed to create organization: {createOrg.error.message}
                </p>
              )}
              <Button
                type="submit"
                className="w-full"
                disabled={createOrg.isPending || !name.trim()}
              >
                {createOrg.isPending ? "Creating..." : "Create organization"}
              </Button>
            </form>
          )}
        </div>
      </Card>
    </div>
  );
}
