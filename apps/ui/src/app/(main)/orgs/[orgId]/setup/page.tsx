"use client";

// Org setup page — onboarding steps 2 (Configure) and 3 (Done), rendered inside
// the shared OnboardingShell.
//
// Configure: provisioning checklist + inline LLM provider setup (provider type +
//   API key). Polling queries drive the checklist; "Skip for now" is preserved.
// Done: shown after the provider form is submitted OR skipped (replaces the old
//   redirect straight to /dashboard). The Done subline is conditional — it only
//   claims a provider is connected when one actually exists; a skip shows a
//   gentle nudge instead. The user proceeds from Done into a real first action.

import { useState, useEffect, useCallback, useRef } from "react";
import { useParams, useRouter } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import {
  Building2,
  Check,
  CircleDot,
  Loader2,
  ArrowRight,
  Plus,
  LayoutGrid,
  Users,
  BookOpen,
} from "lucide-react";
import Link from "next/link";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { getOrganization } from "@/lib/api/organizations";
import { listHarnesses } from "@/lib/api/harnesses";
import { useCreateProvider, useProviders } from "@/hooks/use-providers";
import { usePageTitle } from "@/hooks";
import { ProviderIcon } from "@/components/providers/provider-icon";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import { OnboardingShell } from "@/components/onboarding/onboarding-shell";
import { OSS_ONBOARDING_STEPS } from "@/components/onboarding/steps";
import type { ApiError } from "@/lib/api/client";
import type { DriverId } from "@/lib/api/types";

type SetupProviderType = Extract<DriverId, "openai" | "anthropic">;

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
    label: "Organization created",
    description: "Your new organization has been provisioned",
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

const PROVIDER_OPTIONS: { type: SetupProviderType; label: string }[] = [
  { type: "openai", label: "OpenAI" },
  { type: "anthropic", label: "Anthropic" },
];

// Docs base used elsewhere in the app (see capabilities page).
const DOCS_URL = "https://docs.everruns.com";

function getProviderName(providerType: SetupProviderType): string {
  switch (providerType) {
    case "openai":
      return "OpenAI";
    case "anthropic":
      return "Anthropic";
  }
}

export default function OrgSetupPage() {
  usePageTitle("Org Setup");
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
  const { data: providers = [] } = useProviders();
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
  const [selectedProvider, setSelectedProvider] = useState<SetupProviderType>("openai");
  const [apiKey, setApiKey] = useState("");
  const createProvider = useCreateProvider();
  const [providerError, setProviderError] = useState<string | null>(null);

  // --- Done step state ---
  // `done` flips when the user finishes the provider form or skips. We track
  // whether a provider was actually configured (vs. skipped) so the Done copy
  // never claims a connection that doesn't exist. `hasProvider` covers the
  // already-configured case; `providerJustConfigured` covers a fresh create
  // before the providers query refetches.
  const [done, setDone] = useState(false);
  const [providerJustConfigured, setProviderJustConfigured] = useState(false);
  const providerConnected = hasProvider || providerJustConfigured;

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
      setProviderJustConfigured(true);
      setDone(true);
    } catch {
      setProviderError("Failed to configure provider. Please try again.");
    }
  };

  // --- Loading skeleton ---
  if (orgLoading && !org) {
    return (
      <div className="flex min-h-screen items-center justify-center p-4">
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
      <div className="flex min-h-screen items-center justify-center p-4">
        <Card className="w-full max-w-lg p-8 text-center">
          <div className="flex flex-col items-center gap-4">
            <div className="flex h-12 w-12 items-center justify-center bg-primary/10">
              <Building2 className="h-6 w-6 text-primary" />
            </div>
            <h1 className="text-xl font-semibold">
              {is404 ? "Organization not found" : "Failed to load organization"}
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

  // --- Done step (Frame 6) ---
  if (done) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-background bg-brand-dots p-4">
        <OnboardingShell
          steps={OSS_ONBOARDING_STEPS}
          currentIndex={2}
          stepLabel="Step 3 of 3"
          brand={{
            eyebrow: "Setup complete",
            headline: "You're ready. Spin up an agent that ever runs.",
            features: [
              { label: "Durable execution engine", done: true },
              { label: "Agent harnesses & capabilities", done: true },
              { label: "Session traces & evals", done: true },
            ],
          }}
        >
          <div className="max-w-[460px]">
            <div className="mb-6 flex h-12 w-12 items-center justify-center border border-accent/40 bg-accent/[0.12] text-accent-foreground">
              <Check className="icon-sharp h-6 w-6" strokeWidth={2.2} />
            </div>
            <h1 className="text-[28px] font-semibold tracking-[-0.02em]">You&rsquo;re all set.</h1>
            <p className="mt-3 text-[15px] leading-relaxed text-muted-foreground">
              {providerConnected ? (
                <>
                  {org?.name ?? "Your workspace"} is provisioned and connected to a model provider.
                  Spin up your first agent that ever runs.
                </>
              ) : (
                <>
                  {org?.name ?? "Your workspace"} is ready. You can add a model provider anytime
                  from Settings &mdash; then spin up your first agent.
                </>
              )}
            </p>

            <Link
              href="/agents/new"
              className="mt-6 flex items-center gap-3.5 bg-primary px-5 py-4 text-primary-foreground transition-colors hover:bg-primary/90"
            >
              <span className="flex h-9 w-9 flex-shrink-0 items-center justify-center border border-accent/50 bg-accent/[0.15] text-accent">
                <Plus className="icon-sharp h-[18px] w-[18px]" strokeWidth={2} />
              </span>
              <span className="flex-1">
                <span className="block text-[15px] font-semibold">Create your first agent</span>
                <span className="block text-xs text-primary-foreground/60">
                  Start from a template or a blank harness
                </span>
              </span>
              <ArrowRight className="icon-sharp h-[18px] w-[18px] text-accent" strokeWidth={2} />
            </Link>

            <div className="mt-3.5 flex gap-2.5">
              <Link
                href="/agents/examples"
                className="flex flex-1 flex-col gap-2 border p-3.5 text-foreground transition-colors hover:bg-muted"
              >
                <LayoutGrid
                  className="icon-sharp h-[18px] w-[18px] text-muted-foreground"
                  strokeWidth={1.8}
                />
                <span className="text-[13px] font-medium">Browse templates</span>
              </Link>
              <Link
                href="/settings/members"
                className="flex flex-1 flex-col gap-2 border p-3.5 text-foreground transition-colors hover:bg-muted"
              >
                <Users
                  className="icon-sharp h-[18px] w-[18px] text-muted-foreground"
                  strokeWidth={1.8}
                />
                <span className="text-[13px] font-medium">Invite your team</span>
              </Link>
              <a
                href={DOCS_URL}
                target="_blank"
                rel="noreferrer"
                className="flex flex-1 flex-col gap-2 border p-3.5 text-foreground transition-colors hover:bg-muted"
              >
                <BookOpen
                  className="icon-sharp h-[18px] w-[18px] text-muted-foreground"
                  strokeWidth={1.8}
                />
                <span className="text-[13px] font-medium">Read the docs</span>
              </a>
            </div>
          </div>
        </OnboardingShell>
      </div>
    );
  }

  // --- Configure step (Frame 5): provisioning checklist + provider form ---
  return (
    <div className="flex min-h-screen items-center justify-center bg-background bg-brand-dots p-4">
      <OnboardingShell
        steps={OSS_ONBOARDING_STEPS}
        currentIndex={1}
        stepLabel="Step 2 of 3"
        brand={{
          eyebrow: "Step 2 / 3",
          headline: "Bring your own model. Keys encrypted at rest.",
          features: [
            { label: "Durable execution engine", done: true },
            { label: "Agent harnesses & capabilities", done: true },
            { label: "Session traces & evals" },
          ],
        }}
      >
        <div className="w-full max-w-[460px]">
          <h1 className="text-[22px] font-semibold tracking-[-0.02em]">Setting up {org?.name}</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            We&rsquo;re provisioning your durable workspace. One last thing to finish.
          </p>

          <div className="mt-6 space-y-4">
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
              /* Provider already configured — go straight to the Done step */
              <Button className="w-full" onClick={() => setDone(true)}>
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
                      onClick={() => setDone(true)}
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
                  {createProvider.isPending ? "Configuring..." : "Finish setup"}
                  {!createProvider.isPending && <ArrowRight className="ml-2 h-4 w-4" />}
                </Button>
              </div>
            )}
          </div>
        </div>
      </OnboardingShell>
    </div>
  );
}
