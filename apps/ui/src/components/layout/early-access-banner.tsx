"use client";

// Early-access banner shown above the app shell. Warns that Everruns is still
// stabilizing and links to GitHub. Dismissal is persisted per-user via the
// user-preferences key/value service, so it stays dismissed across devices.
import { useState } from "react";
import { FlaskConical, ArrowRight, X } from "lucide-react";

import { GithubIcon } from "@/components/icons/github-icon";
import { useUserPreference, useSetUserPreference } from "@/hooks/use-user-preferences";

const BANNER_DISMISSED_KEY = "ui.early_access_banner.dismissed";
const GITHUB_URL = "https://github.com/everruns/everruns";

export function EarlyAccessBanner() {
  const { data: preference, isLoading } = useUserPreference(BANNER_DISMISSED_KEY);
  const setPreference = useSetUserPreference();
  // Local flag hides the banner immediately on click, before the write settles.
  const [dismissedLocally, setDismissedLocally] = useState(false);

  const dismissed = dismissedLocally || preference?.value === true;

  // Wait for the stored state before first paint so the banner doesn't flash in
  // for users who already dismissed it.
  if (isLoading || dismissed) {
    return null;
  }

  const handleDismiss = () => {
    setDismissedLocally(true);
    setPreference.mutate({ key: BANNER_DISMISSED_KEY, value: true });
  };

  return (
    <div className="flex h-11 flex-none items-center gap-2.5 border-b border-[hsl(var(--accent)/0.35)] bg-[hsl(var(--accent)/0.12)] px-[18px] text-primary">
      <span className="inline-flex text-accent-foreground">
        <FlaskConical className="h-[15px] w-[15px]" />
      </span>
      <span className="text-[13px] tracking-[-0.01em]">
        <strong className="font-semibold">Early access.</strong> Everruns is still stabilizing —
        expect breaking changes and rough edges.
      </span>
      <span className="flex-1" />
      <a
        href={GITHUB_URL}
        target="_blank"
        rel="noopener noreferrer"
        className="inline-flex items-center gap-1 whitespace-nowrap text-[13px] font-medium text-primary hover:underline hover:underline-offset-[3px]"
      >
        <GithubIcon className="h-[14px] w-[14px]" /> Follow along on GitHub{" "}
        <ArrowRight className="h-[13px] w-[13px]" />
      </a>
      <button
        type="button"
        aria-label="Dismiss"
        onClick={handleDismiss}
        className="inline-flex h-[26px] w-[26px] cursor-pointer items-center justify-center border-0 bg-transparent text-primary/70 hover:bg-[hsl(var(--accent)/0.18)] hover:text-primary"
      >
        <X className="h-[15px] w-[15px]" />
      </button>
    </div>
  );
}
