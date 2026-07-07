/**
 * OSS-owned zero-organization onboarding surface (step 1 of the onboarding arc).
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
 *  - `suggestedName` lets a wrapper prefill the org name (SaaS derives this from
 *    the user's work-email domain). OSS passes nothing — self-host has no email
 *    domain to derive from. When provided, the input is prefilled and an
 *    "Auto-filled from …" chip is shown; the field stays fully editable.
 *
 * Renders inside `OnboardingShell` (Frame 4 of the onboarding design): explainer
 * copy, the org-name form, and an informational "invite teammates later" card.
 */
"use client";

import { useState, type ReactNode } from "react";
import { useRouter } from "next/navigation";
import { ArrowRight, Check, Loader2, Users } from "lucide-react";
import { useCreateOrganization } from "@/hooks/use-organizations";
import { useOrg } from "@/providers/org-provider";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { OnboardingShell } from "./onboarding-shell";
import { useOnboardingArc } from "./onboarding-arc-context";

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
  /**
   * Optional pre-fill for the org-name input (e.g. derived from a work-email
   * domain by a SaaS wrapper). Shows an "Auto-filled from …" chip; still fully
   * editable. OSS passes nothing.
   */
  suggestedName?: string;
}

export function ZeroOrgOnboarding({
  usePolicy = useReadyZeroOrgPolicy,
  useCreateOrg = useCreateOrganization,
  suggestedName,
}: ZeroOrgOnboardingProps = {}) {
  const router = useRouter();
  const { setCurrentOrg } = useOrg();
  const createOrg = useCreateOrg();
  const policy = usePolicy();
  // Arc position: this surface is the first OSS step ("Organisation"); a
  // wrapper-provided arc (e.g. SaaS with a prepended "Verify") shifts the
  // stepper index and labels via context.
  const arc = useOnboardingArc();
  const [name, setName] = useState(suggestedName ?? "");
  // Show the auto-filled chip only while the prefilled value is unchanged.
  const showAutoFill = !!suggestedName && name === suggestedName;

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
      <OnboardingShell
        steps={arc.steps}
        currentIndex={arc.stepIndex(0)}
        stepLabel={arc.stepLabel(0)}
        brand={{
          eyebrow: `Step ${arc.stepIndex(0) + 1} / ${arc.steps.length}`,
          headline: "Your workspace for agents, harnesses, and durable runs.",
          // "What you'll get" checklist — fills in on later steps.
          features: [
            { label: "Durable execution engine" },
            { label: "Agent harnesses & capabilities" },
            { label: "Session traces & evals" },
          ],
        }}
      >
        <div className="max-w-[420px]">
          <h1 className="text-[28px] font-semibold tracking-[-0.02em]">Create your organisation</h1>
          <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
            An organisation is your team&rsquo;s shared workspace. Agents, harnesses, sessions, and
            settings all live here.
          </p>

          <div className="mt-7">
            {policy.status === "loading" ? (
              <div className="flex justify-center py-6">
                <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
              </div>
            ) : policy.status === "blocked" ? (
              <div className="space-y-4" role="alert">
                <h2 className="text-base font-semibold">{policy.title}</h2>
                {policy.body && <div className="text-sm text-muted-foreground">{policy.body}</div>}
                {policy.actions && <div className="flex gap-2">{policy.actions}</div>}
              </div>
            ) : (
              <form onSubmit={handleSubmit} className="space-y-4">
                <div className="space-y-2">
                  <Label htmlFor="onboarding-org-name">Organisation name</Label>
                  <Input
                    id="onboarding-org-name"
                    value={name}
                    onChange={(event) => setName(event.target.value)}
                    placeholder="Acme Inc."
                    required
                  />
                  {showAutoFill && (
                    <span className="inline-flex items-center gap-1.5 border border-accent/40 bg-accent/[0.12] px-2 py-1 font-mono text-[11px] text-accent-foreground">
                      <Check className="icon-sharp h-3 w-3" strokeWidth={2.5} />
                      Auto-filled from {suggestedName}
                    </span>
                  )}
                </div>

                {/* Informational only — invites happen later from Settings. */}
                <div className="flex items-center gap-3 border p-3.5">
                  <span className="flex h-5 w-5 flex-shrink-0 items-center justify-center bg-primary text-primary-foreground">
                    <Users className="icon-sharp h-3 w-3" strokeWidth={2.5} />
                  </span>
                  <div>
                    <div className="text-[13px] font-medium">Invite teammates later</div>
                    <div className="text-xs text-muted-foreground">
                      You can add people anytime from Settings.
                    </div>
                  </div>
                </div>

                {createOrg.isError && (
                  // Generic copy only — server error strings are not for
                  // rendering (TM-AUTH-019 discipline applies UI-wide).
                  <p role="alert" className="text-sm text-destructive">
                    Failed to create organisation. Please try again.
                  </p>
                )}
                <Button
                  type="submit"
                  className="w-full"
                  disabled={createOrg.isPending || !name.trim()}
                >
                  {createOrg.isPending ? "Creating..." : "Create organisation"}
                  {!createOrg.isPending && <ArrowRight className="ml-2 h-4 w-4" />}
                </Button>
              </form>
            )}
          </div>
        </div>
      </OnboardingShell>
    </div>
  );
}
