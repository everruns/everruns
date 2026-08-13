"use client";

// Org setup page — onboarding steps 2 (Configure) and 3 (Done), rendered inside
// the shared OnboardingShell.
//
// Configure: provisioning checklist + inline LLM provider setup (provider type +
//   API key). Polling queries drive the checklist; "Skip for now" is preserved.
// Done: shown after the provider form is submitted OR skipped (replaces the old
//   redirect straight to /chats). The Done subline is conditional — it only
//   claims a provider is connected when one actually exists; a skip shows a
//   gentle nudge instead. The user proceeds from Done into a real first action.

import { useState, useEffect, useCallback, useRef } from "react";
import { useParams, useRouter } from "next/navigation";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Building2,
  Check,
  CircleDot,
  Clock,
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
import { getOrganization, completeOrgOnboarding } from "@/lib/api/organizations";
import { listHarnesses } from "@/lib/api/harnesses";
import { useCreateProvider, useProviders } from "@/hooks/use-providers";
import { usePageTitle } from "@/hooks";
import { ProviderIcon } from "@/components/providers/provider-icon";
import { queryKeys } from "@/lib/query-keys";
import { useOrg } from "@/providers/org-provider";
import { OnboardingShell } from "@/components/onboarding/onboarding-shell";
import { useOnboardingArc } from "@/components/onboarding/onboarding-arc-context";
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

// Labels follow the onboarding-arc copy: the last item is an action ("first
// provider" — more can be added later), rendered in-progress until a provider
// actually exists rather than falsely ticking off.
const SETUP_STEPS: SetupStep[] = [
  {
    label: "Organisation created",
    description: "Your new organisation has been provisioned",
    check: (ctx) => ctx.orgLoaded,
  },
  {
    label: "Harnesses provisioned",
    description: "Built-in harnesses are ready",
    check: (ctx) => ctx.harnessCount > 0,
  },
  {
    label: "Settings initialised",
    description: "Default and base harnesses have been assigned",
    check: (ctx) => !!ctx.defaultHarnessId && !!ctx.baseHarnessId,
  },
  {
    label: "Connect your first provider",
    description: "Add a model provider to finish setup",
    // Always part of the animated sequence — actual config happens inline
    check: () => true,
  },
];

const STEP_DELAY_MS = 400;

const PROVIDER_OPTIONS: { type: SetupProviderType; label: string; models: string }[] = [
  { type: "openai", label: "OpenAI", models: "GPT-5.5, o-series" },
  { type: "anthropic", label: "Anthropic", models: "Claude Opus, Sonnet" },
];

// The setup form features the two most common providers; the rest of the
// driver catalog stays one click away after setup so the pair never reads as
// "only two models exist".
const MORE_PROVIDERS = ["Gemini", "Azure OpenAI", "Bedrock"];
const MORE_PROVIDERS_EXTRA = 5; // openrouter, openai_completions, mai, fireworks, meta

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
  const queryClient = useQueryClient();
  const { currentOrg, organizations, setCurrentOrg, isSwitching } = useOrg();
  // Arc position: Configure is OSS step 1, Done is OSS step 2. A wrapper arc
  // (e.g. SaaS with a prepended "Verify") shifts indices/labels via context so
  // the stepper stays continuous across the whole journey.
  const arc = useOnboardingArc();

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

  // Durably mark onboarding complete (finish OR skip), then reveal the Done
  // step. Best-effort: on failure we still proceed to Done — the resume redirect
  // may re-show setup on the next entry, which is acceptable. On success we seed
  // the org-detail cache so the resume redirect immediately sees completion and
  // does not bounce the user back here after they navigate away.
  const finishOnboarding = useCallback(async () => {
    try {
      const updated = await completeOrgOnboarding(orgId);
      queryClient.setQueryData(queryKeys.organizations.detail(orgId), updated);
    } catch {
      // Intentionally ignored — completion is best-effort.
    }
    setDone(true);
  }, [orgId, queryClient]);

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
      await finishOnboarding();
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
            {!is404 && (
              <p role="alert" className="text-sm text-muted-foreground">
                Something went wrong loading this organisation. Please retry.
              </p>
            )}
            <Button variant="outline" onClick={() => router.push("/chats")}>
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
          steps={arc.steps}
          currentIndex={arc.stepIndex(2)}
          stepLabel={arc.stepLabel(2)}
          brand={{
            eyebrow: "Setup complete",
            headline:
              "Deploy your first agent and keep it running — through restarts, deploys, and outages.",
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
                  Deploy your first agent and keep it running &mdash; through restarts, deploys, and
                  outages.
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
        steps={arc.steps}
        currentIndex={arc.stepIndex(1)}
        stepLabel={arc.stepLabel(1)}
        brand={{
          eyebrow: `Step ${arc.stepIndex(1) + 1} / ${arc.steps.length}`,
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
              const isProviderStep = i === SETUP_STEPS.length - 1;

              // The provider step is an action, not a provisioning fact: it
              // holds a gold in-progress badge until a provider actually
              // exists instead of falsely ticking off with the animation.
              if (isProviderStep && animated && !providerConnected) {
                return (
                  <div key={step.label} className="flex items-center gap-3">
                    <div className="flex h-[26px] w-[26px] shrink-0 items-center justify-center border border-accent bg-accent/[0.14] text-accent-foreground">
                      <Clock className="icon-sharp h-[13px] w-[13px]" strokeWidth={2} />
                    </div>
                    <div>
                      <p className="text-sm font-semibold">{step.label}</p>
                      <p className="text-xs text-accent-foreground">{step.description}</p>
                    </div>
                  </div>
                );
              }

              // Completed and animated — sharp navy check badge (0px radius,
              // matches the stepper)
              if (passed && animated) {
                return (
                  <div key={step.label} className="flex items-center gap-3">
                    <div className="flex h-[26px] w-[26px] shrink-0 items-center justify-center border border-primary bg-primary text-primary-foreground">
                      <Check className="icon-sharp h-[13px] w-[13px]" strokeWidth={2.5} />
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
                    <div className="flex h-[26px] w-[26px] shrink-0 items-center justify-center">
                      <Loader2 className="h-4 w-4 animate-spin text-primary" />
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
                  <div className="flex h-[26px] w-[26px] shrink-0 items-center justify-center">
                    <CircleDot className="icon-sharp h-4 w-4 text-muted-foreground/50" />
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
              <Button className="w-full" onClick={() => void finishOnboarding()}>
                Continue
                <ArrowRight className="ml-2 h-4 w-4" />
              </Button>
            ) : (
              <div className="space-y-5">
                {/* Provider type selector */}
                <div>
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
                          className={`relative flex items-center gap-3 border p-3.5 text-left transition-colors ${
                            isSelected
                              ? "border-accent bg-accent/[0.07]"
                              : "border-border hover:border-primary/40"
                          }`}
                        >
                          <ProviderIcon providerType={opt.type} />
                          <span className="min-w-0">
                            <span
                              className={`block text-sm ${isSelected ? "font-semibold" : "font-medium"}`}
                            >
                              {opt.label}
                            </span>
                            <span className="block truncate text-xs text-muted-foreground">
                              {opt.models}
                            </span>
                          </span>
                          {/* Sharp gold selection badge (0px radius) */}
                          {isSelected && (
                            <span className="absolute right-2.5 top-2.5 flex h-[18px] w-[18px] items-center justify-center bg-accent text-accent-foreground">
                              <Check className="icon-sharp h-[11px] w-[11px]" strokeWidth={3} />
                            </span>
                          )}
                        </button>
                      );
                    })}
                  </div>

                  {/* Breadth strip: the two featured cards must not read as
                      the whole catalog. Informational only — the full picker
                      lives in Settings → Providers after setup. */}
                  <div className="mt-3 flex flex-wrap items-center gap-2">
                    <span className="text-xs text-muted-foreground">Also supported</span>
                    {MORE_PROVIDERS.map((name) => (
                      <span
                        key={name}
                        className="border px-2 py-[3px] text-xs text-muted-foreground"
                      >
                        {name}
                      </span>
                    ))}
                    <span className="text-xs text-muted-foreground">
                      + {MORE_PROVIDERS_EXTRA} more in Settings after setup
                    </span>
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
                    <p role="alert" className="mt-1.5 text-xs text-destructive">
                      {providerError}
                    </p>
                  )}
                </div>

                {/* Footer row: skip escape hatch left, primary action right */}
                <div className="flex items-center justify-between pt-1">
                  <button
                    type="button"
                    onClick={() => void finishOnboarding()}
                    disabled={createProvider.isPending}
                    className="text-sm text-muted-foreground transition-colors hover:text-foreground disabled:opacity-50"
                  >
                    Skip for now
                  </button>
                  <Button onClick={handleContinue} disabled={createProvider.isPending}>
                    {createProvider.isPending ? "Configuring..." : "Finish setup"}
                    {!createProvider.isPending && <ArrowRight className="ml-2 h-4 w-4" />}
                  </Button>
                </div>
              </div>
            )}
          </div>
        </div>
      </OnboardingShell>
    </div>
  );
}
