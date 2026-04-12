"use client";

// Org setup page — shown after org creation to confirm provisioning.
// Includes inline LLM provider setup (provider type + API key) as the final step.

import { useState, useEffect, useCallback, useRef } from "react";
import { useParams, useRouter } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { Building2, Check, CircleDot, Loader2, ArrowRight } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { getOrganization } from "@/lib/api/organizations";
import { listHarnesses } from "@/lib/api/harnesses";
import { useCreateLlmProvider, useLlmProviders } from "@/hooks/use-llm-providers";
import { ProviderIcon } from "@/components/providers/provider-icon";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import type { ApiError } from "@/lib/api/client";
import type { LlmProviderType } from "@/lib/api/types";

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
  {
    label: "LLM provider configured",
    description: "API provider and credentials set up",
    // Always shows as part of the animated sequence — actual config happens inline
    check: () => true,
  },
];

const STEP_DELAY_MS = 400;

const PROVIDER_OPTIONS: { type: LlmProviderType; label: string }[] = [
  { type: "openai", label: "OpenAI" },
  { type: "anthropic", label: "Anthropic" },
];

function getProviderName(providerType: LlmProviderType): string {
  switch (providerType) {
    case "openai":
    case "openai_completions":
      return "OpenAI";
    case "anthropic":
      return "Anthropic";
    case "gemini":
      return "Gemini";
    default:
      return providerType;
  }
}

export default function OrgSetupPage() {
  const { orgId } = useParams<{ orgId: string }>();
  const router = useRouter();
  const { currentOrg, organizations, setCurrentOrg, isSwitching } = useOrg();

  const {
    data: org,
    isLoading: orgLoading,
    isError: orgError,
    error: orgErrorDetail,
  } = useQuery({
    queryKey: queryKeys.organizations.detail(orgId),
    queryFn: () => getOrganization(orgId),
    staleTime: 30_000,
    refetchInterval: (query) => {
      const data = query.state.data;
      return data?.default_harness_id && data?.base_harness_id ? false : 5_000;
    },
  });

  // Gate harness query on org cookie being set for the correct org
  const orgReady = currentOrg?.public_id === orgId && !isSwitching;

  const { data: harnesses = [] } = useQuery({
    queryKey: [...queryKeys.harnesses.all, orgId],
    queryFn: () => listHarnesses(),
    enabled: orgReady,
    staleTime: 30_000,
    refetchInterval: (query) => {
      const data = query.state.data ?? [];
      return data.length > 0 ? false : 5_000;
    },
  });

  // Check if a provider already exists (skip the form if so)
  const { data: providers = [] } = useLlmProviders();
  const hasProvider = providers.length > 0;

  // Set current org once loaded — derive role from membership list when available
  useEffect(() => {
    if (org) {
      const membership = organizations.find((o) => o.public_id === org.id);
      setCurrentOrg({ public_id: org.id, name: org.name, role: membership?.role ?? "owner" });
    }
  }, [org, organizations, setCurrentOrg]);

  const stepContext: StepContext = {
    orgLoaded: !!org,
    harnessCount: harnesses.length,
    defaultHarnessId: org?.default_harness_id ?? null,
    baseHarnessId: org?.base_harness_id ?? null,
  };

  const allReady = SETUP_STEPS.every((s) => s.check(stepContext));

  // Animated step completion — reveal one at a time, with cleanup on unmount
  const [completedCount, setCompletedCount] = useState(0);
  const timersRef = useRef<ReturnType<typeof setTimeout>[]>([]);

  useEffect(() => {
    const timers = timersRef.current;
    return () => {
      for (const id of timers) clearTimeout(id);
    };
  }, []);

  const animateSteps = useCallback(() => {
    let idx = 0;
    const tick = () => {
      idx += 1;
      setCompletedCount(idx);
      if (idx < SETUP_STEPS.length) {
        timersRef.current.push(setTimeout(tick, STEP_DELAY_MS));
      }
    };
    timersRef.current.push(setTimeout(tick, STEP_DELAY_MS));
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

  // --- LLM provider form state ---
  const [selectedProvider, setSelectedProvider] = useState<LlmProviderType>("openai");
  const [apiKey, setApiKey] = useState("");
  const createProvider = useCreateLlmProvider();
  const [providerError, setProviderError] = useState<string | null>(null);

  const handleContinue = async () => {
    if (!apiKey.trim()) {
      setProviderError("Please enter an API key");
      return;
    }
    setProviderError(null);
    try {
      await createProvider.mutateAsync({
        name: getProviderName(selectedProvider),
        provider_type: selectedProvider,
        api_key: apiKey,
      });
      router.push("/dashboard");
    } catch {
      setProviderError("Failed to configure provider. Please try again.");
    }
  };

  // --- Loading skeleton ---
  if (orgLoading && !org) {
    return (
      <div className="flex min-h-[60vh] items-center justify-center p-4">
        <Card className="w-full max-w-lg p-8">
          <div className="flex flex-col items-center gap-4">
            <Skeleton className="h-12 w-12" />
            <Skeleton className="h-6 w-48" />
            <Skeleton className="h-4 w-64" />
          </div>
          <div className="mt-8 space-y-4">
            <Skeleton className="h-12 w-full" />
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
    const is404 = (orgErrorDetail as ApiError)?.status === 404;
    return (
      <div className="flex min-h-[60vh] items-center justify-center p-4">
        <Card className="w-full max-w-lg p-8 text-center">
          <div className="flex flex-col items-center gap-4">
            <div className="flex h-12 w-12 items-center justify-center bg-primary/10">
              <Building2 className="h-6 w-6 text-primary" />
            </div>
            <h1 className="text-xl font-semibold">
              {is404 ? "Organisation not found" : "Failed to load organisation"}
            </h1>
            {!is404 && orgErrorDetail?.message && (
              <p className="text-sm text-muted-foreground">{orgErrorDetail.message}</p>
            )}
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
          <div className="flex h-12 w-12 items-center justify-center bg-primary/10">
            <Building2 className="h-6 w-6 text-primary" />
          </div>
          <h1 className="text-xl font-semibold">Setting up {org?.name}</h1>
          <p className="text-sm text-muted-foreground">
            Your organisation is being configured with everything you need to get started.
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
                    <p className="text-xs text-muted-foreground">{step.description}</p>
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
                    <p className="text-xs text-muted-foreground">{step.description}</p>
                  </div>
                </div>
              );
            }

            // Pending
            return (
              <div key={step.label} className="flex items-center gap-3 opacity-40">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center">
                  <CircleDot className="h-5 w-5 text-muted-foreground/50" />
                </div>
                <div>
                  <p className="text-sm font-medium">{step.label}</p>
                  <p className="text-xs text-muted-foreground">{step.description}</p>
                </div>
              </div>
            );
          })}
        </div>

        {/* LLM Provider selection — shown after step animation completes */}
        <div
          className={`mt-8 transition-opacity duration-500 ${allAnimated ? "opacity-100" : "opacity-0 pointer-events-none"}`}
        >
          {hasProvider ? (
            /* Provider already configured — go straight to dashboard */
            <Button className="w-full" onClick={() => router.push("/dashboard")}>
              Continue
              <ArrowRight className="ml-2 h-4 w-4" />
            </Button>
          ) : (
            <div className="space-y-6">
              {/* Provider type selector */}
              <div>
                <h2 className="text-sm font-semibold mb-3">Select your LLM provider</h2>
                <div className="grid grid-cols-2 gap-3">
                  {PROVIDER_OPTIONS.map((opt) => {
                    const isSelected = selectedProvider === opt.type;
                    return (
                      <button
                        key={opt.type}
                        type="button"
                        onClick={() => {
                          setSelectedProvider(opt.type);
                          setProviderError(null);
                        }}
                        className={`relative flex flex-col items-center gap-2 border-2 p-4 transition-colors ${
                          isSelected
                            ? "border-primary bg-primary/5"
                            : "border-border hover:border-primary/40"
                        }`}
                      >
                        <ProviderIcon providerType={opt.type} size="lg" />
                        <span className="text-sm font-medium">{opt.label}</span>
                        {/* Radio indicator */}
                        <div
                          className={`h-4 w-4 rounded-full border-2 transition-colors ${
                            isSelected ? "border-primary" : "border-muted-foreground/40"
                          }`}
                        >
                          {isSelected && (
                            <div className="m-[2px] h-2 w-2 rounded-full bg-primary" />
                          )}
                        </div>
                      </button>
                    );
                  })}
                </div>

                {/* Skip link */}
                <div className="mt-3 text-center">
                  <button
                    type="button"
                    onClick={() => router.push("/dashboard")}
                    className="text-sm text-muted-foreground underline underline-offset-4 hover:text-foreground transition-colors"
                  >
                    Skip for now
                  </button>
                </div>
              </div>

              {/* API Key input */}
              <div>
                <label htmlFor="setup-api-key" className="text-sm font-semibold">
                  API Key
                </label>
                <Input
                  id="setup-api-key"
                  type="password"
                  className="mt-1.5"
                  value={apiKey}
                  onChange={(e: React.ChangeEvent<HTMLInputElement>) => {
                    setApiKey(e.target.value);
                    setProviderError(null);
                  }}
                  placeholder={`Enter your ${getProviderName(selectedProvider)} API key`}
                />
                <p className="mt-1.5 text-xs text-muted-foreground">
                  Your API key will be stored securely and encrypted
                </p>
                {providerError && (
                  <p className="mt-1.5 text-xs text-destructive">{providerError}</p>
                )}
              </div>

              {/* Continue button */}
              <Button
                className="w-full"
                onClick={handleContinue}
                disabled={createProvider.isPending}
              >
                {createProvider.isPending ? "Configuring..." : "Continue"}
                {!createProvider.isPending && <ArrowRight className="ml-2 h-4 w-4" />}
              </Button>
            </div>
          )}
        </div>
      </Card>
    </div>
  );
}
