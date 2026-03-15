"use client";

// Org setup page shown after org creation.
// Displays step-by-step progress (harness init, default settings)
// with a "Go to dashboard" button when complete.

import { useEffect, useState } from "react";
import { useParams, useRouter } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { getOrganization } from "@/lib/api/organizations";
import { listHarnesses } from "@/lib/api/harnesses";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import { Building2, Check, CircleDot, Loader2, ArrowRight } from "lucide-react";

type StepStatus = "pending" | "in_progress" | "completed";

interface SetupStep {
  id: string;
  label: string;
  description: string;
}

const SETUP_STEPS: SetupStep[] = [
  {
    id: "org_created",
    label: "Organisation created",
    description: "Your new organisation has been provisioned",
  },
  {
    id: "harnesses_init",
    label: "Harnesses initialised",
    description: "Built-in harnesses (Base, Generic, Platform Chat) are ready",
  },
  {
    id: "defaults_configured",
    label: "Default settings configured",
    description: "Default and base harnesses have been assigned",
  },
];

export default function OrgSetupPage() {
  const params = useParams<{ orgId: string }>();
  const router = useRouter();
  const { setCurrentOrg, organizations } = useOrg();
  const orgId = params.orgId;

  // Track which step is currently animating
  const [visibleStep, setVisibleStep] = useState(0);

  // Fetch org details to confirm it exists
  const {
    data: org,
    isLoading: orgLoading,
    error: orgError,
  } = useQuery({
    queryKey: queryKeys.organizations.detail(orgId),
    queryFn: () => getOrganization(orgId),
    enabled: !!orgId,
    staleTime: 30000,
  });

  // Fetch harnesses for this org (via current org context)
  const { data: harnesses = [] } = useQuery({
    queryKey: [...queryKeys.harnesses.all, orgId],
    queryFn: () => listHarnesses(),
    enabled: !!org,
    staleTime: 30000,
  });

  // Derive step statuses from real data
  const orgReady = !!org;
  const harnessesReady = harnesses.length > 0;
  const defaultsReady = !!org?.default_harness_id && !!org?.base_harness_id;
  const allDone = orgReady && harnessesReady && defaultsReady;

  // Ensure this org is set as current so harness queries work
  useEffect(() => {
    if (!orgId || organizations.length === 0) return;
    const membership = organizations.find((o) => o.public_id === orgId);
    if (membership) {
      setCurrentOrg(membership);
    }
  }, [orgId, organizations, setCurrentOrg]);

  // Animate steps completing one by one
  useEffect(() => {
    if (!allDone) return;

    const timers: ReturnType<typeof setTimeout>[] = [];
    for (let i = 0; i <= SETUP_STEPS.length; i++) {
      timers.push(
        setTimeout(
          () => {
            setVisibleStep(i + 1);
          },
          400 * (i + 1),
        ),
      );
    }
    return () => timers.forEach(clearTimeout);
  }, [allDone]);

  const getStepStatus = (index: number): StepStatus => {
    if (visibleStep > index) return "completed";
    if (visibleStep === index && allDone) return "in_progress";
    return "pending";
  };

  const allStepsVisible = visibleStep > SETUP_STEPS.length;

  if (orgError) {
    return (
      <div className="flex items-center justify-center min-h-[60vh]">
        <Card className="p-8 text-center max-w-md">
          <Building2 className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
          <h2 className="text-lg font-semibold mb-2">Organisation not found</h2>
          <p className="text-sm text-muted-foreground mb-4">
            The organisation could not be loaded. It may not exist or you may not have access.
          </p>
          <Button variant="outline" onClick={() => router.push("/dashboard")}>
            Go to dashboard
          </Button>
        </Card>
      </div>
    );
  }

  if (orgLoading) {
    return (
      <div className="flex items-center justify-center min-h-[60vh]">
        <Card className="p-8 w-full max-w-lg">
          <Skeleton className="h-8 w-48 mb-2" />
          <Skeleton className="h-4 w-64 mb-8" />
          <div className="space-y-6">
            <Skeleton className="h-16 w-full" />
            <Skeleton className="h-16 w-full" />
            <Skeleton className="h-16 w-full" />
          </div>
        </Card>
      </div>
    );
  }

  return (
    <div className="flex items-center justify-center min-h-[60vh]">
      <Card className="p-8 w-full max-w-lg">
        <div className="flex items-center gap-3 mb-2">
          <div className="p-2 rounded-lg bg-primary/10">
            <Building2 className="h-5 w-5 text-primary" />
          </div>
          <h2 className="text-xl font-semibold">Setting up {org?.name}</h2>
        </div>
        <p className="text-sm text-muted-foreground mb-8">
          Your organisation is being configured with everything you need to get started.
        </p>

        <div className="space-y-4">
          {SETUP_STEPS.map((step, index) => {
            const status = getStepStatus(index);
            return (
              <div
                key={step.id}
                className={`flex items-start gap-3 transition-opacity duration-300 ${
                  status === "pending" ? "opacity-40" : "opacity-100"
                }`}
              >
                <div className="mt-0.5">
                  {status === "completed" ? (
                    <div className="h-6 w-6 rounded-full bg-primary flex items-center justify-center">
                      <Check className="h-3.5 w-3.5 text-primary-foreground" />
                    </div>
                  ) : status === "in_progress" ? (
                    <Loader2 className="h-6 w-6 text-primary animate-spin" />
                  ) : (
                    <CircleDot className="h-6 w-6 text-muted-foreground/50" />
                  )}
                </div>
                <div>
                  <p
                    className={`text-sm font-medium ${
                      status === "completed" ? "text-foreground" : "text-muted-foreground"
                    }`}
                  >
                    {step.label}
                  </p>
                  <p className="text-xs text-muted-foreground">{step.description}</p>
                </div>
              </div>
            );
          })}
        </div>

        <div
          className={`mt-8 transition-opacity duration-500 ${
            allStepsVisible ? "opacity-100" : "opacity-0 pointer-events-none"
          }`}
        >
          <Button className="w-full" onClick={() => router.push("/dashboard")}>
            Go to dashboard
            <ArrowRight className="h-4 w-4 ml-2" />
          </Button>
        </div>
      </Card>
    </div>
  );
}
