"use client";

// Org setup page — shown after org creation to confirm provisioning.
// Extensible: future steps (e.g. LLM key configuration) can be added to SETUP_STEPS.

import { useState, useEffect, useCallback } from "react";
import { useParams, useRouter } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { Building2, Check, CircleDot, Loader2, ArrowRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { getOrganization } from "@/lib/api/organizations";
import { listHarnesses } from "@/lib/api/harnesses";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";

interface SetupStep {
  label: string;
  description: string;
  check: (ctx: StepContext) => boolean;
}

interface StepContext {
  orgLoaded: boolean;
  harnessCount: number;
  defaultHarnessId: string | null;
  baseHarnessId: string | null;
}

const SETUP_STEPS: SetupStep[] = [
  {
    label: "Organisation created",
    description: "Your new organisation has been provisioned",
    check: (ctx) => ctx.orgLoaded,
  },
  {
    label: "Harnesses initialised",
    description: "Built-in harnesses are ready",
    check: (ctx) => ctx.harnessCount > 0,
  },
  {
    label: "Default settings configured",
    description: "Default and base harnesses have been assigned",
    check: (ctx) => !!ctx.defaultHarnessId && !!ctx.baseHarnessId,
  },
];

const STEP_DELAY_MS = 400;

export default function OrgSetupPage() {
  const { orgId } = useParams<{ orgId: string }>();
  const router = useRouter();
  const { setCurrentOrg } = useOrg();

  const {
    data: org,
    isLoading: orgLoading,
    isError: orgError,
  } = useQuery({
    queryKey: queryKeys.organizations.detail(orgId),
    queryFn: () => getOrganization(orgId),
    staleTime: 30_000,
    refetchInterval: 5_000,
  });

  const { data: harnesses = [] } = useQuery({
    queryKey: queryKeys.harnesses.all,
    queryFn: () => listHarnesses(),
    staleTime: 30_000,
    refetchInterval: 5_000,
  });

  // Set current org once loaded
  useEffect(() => {
    if (org) {
      setCurrentOrg({ public_id: org.id, name: org.name, role: "owner" });
    }
  }, [org, setCurrentOrg]);

  const stepContext: StepContext = {
    orgLoaded: !!org,
    harnessCount: harnesses.length,
    defaultHarnessId: org?.default_harness_id ?? null,
    baseHarnessId: org?.base_harness_id ?? null,
  };

  const allReady = SETUP_STEPS.every((s) => s.check(stepContext));

  // Animated step completion — reveal one at a time
  const [completedCount, setCompletedCount] = useState(0);

  const animateSteps = useCallback(() => {
    let idx = 0;
    const tick = () => {
      idx += 1;
      setCompletedCount(idx);
      if (idx < SETUP_STEPS.length) {
        setTimeout(tick, STEP_DELAY_MS);
      }
    };
    setTimeout(tick, STEP_DELAY_MS);
  }, []);

  // Trigger animation once all checks pass
  const [animationStarted, setAnimationStarted] = useState(false);
  useEffect(() => {
    if (allReady && !animationStarted) {
      setAnimationStarted(true);
      animateSteps();
    }
  }, [allReady, animationStarted, animateSteps]);

  const allAnimated = completedCount >= SETUP_STEPS.length;

  // --- Loading skeleton ---
  if (orgLoading && !org) {
    return (
      <div className="flex min-h-[60vh] items-center justify-center p-4">
        <Card className="w-full max-w-lg p-8">
          <div className="flex flex-col items-center gap-4">
            <Skeleton className="h-12 w-12 rounded-xl" />
            <Skeleton className="h-6 w-48" />
            <Skeleton className="h-4 w-64" />
          </div>
          <div className="mt-8 space-y-4">
            <Skeleton className="h-12 w-full" />
            <Skeleton className="h-12 w-full" />
            <Skeleton className="h-12 w-full" />
          </div>
        </Card>
      </div>
    );
  }

  // --- Error state ---
  if (orgError) {
    return (
      <div className="flex min-h-[60vh] items-center justify-center p-4">
        <Card className="w-full max-w-lg p-8 text-center">
          <div className="flex flex-col items-center gap-4">
            <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-primary/10">
              <Building2 className="h-6 w-6 text-primary" />
            </div>
            <h1 className="text-xl font-semibold">Organisation not found</h1>
            <Button variant="outline" onClick={() => router.push("/dashboard")}>
              Go to dashboard
            </Button>
          </div>
        </Card>
      </div>
    );
  }

  // --- Setup progress ---
  return (
    <div className="flex min-h-[60vh] items-center justify-center p-4">
      <Card className="w-full max-w-lg p-8">
        <div className="flex flex-col items-center gap-2 text-center">
          <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-primary/10">
            <Building2 className="h-6 w-6 text-primary" />
          </div>
          <h1 className="text-xl font-semibold">Setting up {org?.name}</h1>
          <p className="text-sm text-muted-foreground">
            Your organisation is being configured with everything you need to
            get started.
          </p>
        </div>

        <div className="mt-8 space-y-4">
          {SETUP_STEPS.map((step, i) => {
            const passed = step.check(stepContext);
            const animated = i < completedCount;

            // Completed and animated
            if (passed && animated) {
              return (
                <div key={step.label} className="flex items-center gap-3">
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-primary text-primary-foreground">
                    <Check className="h-4 w-4" />
                  </div>
                  <div>
                    <p className="text-sm font-medium">{step.label}</p>
                    <p className="text-xs text-muted-foreground">
                      {step.description}
                    </p>
                  </div>
                </div>
              );
            }

            // In progress (first non-animated step when checks are passing)
            if (passed && !animated) {
              return (
                <div key={step.label} className="flex items-center gap-3">
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center">
                    <Loader2 className="h-5 w-5 animate-spin text-primary" />
                  </div>
                  <div>
                    <p className="text-sm font-medium">{step.label}</p>
                    <p className="text-xs text-muted-foreground">
                      {step.description}
                    </p>
                  </div>
                </div>
              );
            }

            // Pending
            return (
              <div
                key={step.label}
                className="flex items-center gap-3 opacity-40"
              >
                <div className="flex h-8 w-8 shrink-0 items-center justify-center">
                  <CircleDot className="h-5 w-5 text-muted-foreground/50" />
                </div>
                <div>
                  <p className="text-sm font-medium">{step.label}</p>
                  <p className="text-xs text-muted-foreground">
                    {step.description}
                  </p>
                </div>
              </div>
            );
          })}
        </div>

        <div
          className={`mt-8 transition-opacity duration-500 ${allAnimated ? "opacity-100" : "opacity-0 pointer-events-none"}`}
        >
          <Button className="w-full" onClick={() => router.push("/dashboard")}>
            Go to dashboard
            <ArrowRight className="ml-2 h-4 w-4" />
          </Button>
        </div>
      </Card>
    </div>
  );
}
