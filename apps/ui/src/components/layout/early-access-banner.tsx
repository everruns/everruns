"use client";

// Early-access banner shown above the app shell. Warns that Everruns is still
// stabilizing and links to GitHub. Dismissal is persisted per-user via the
// user-preferences key/value service, so it stays dismissed across devices.
//
// EVE-794: the banner previously returned null while the dismissed preference
// was loading, then inserted a 44px (h-11) row after the request resolved for
// undismissed users — shifting the entire app shell down (CLS ~0.024). Reserving
// the slot during loading and collapsing on resolve would instead shift the
// shell for dismissed users, so neither half-measure is enough.
//
// Instead the dismissal decision is available synchronously at first paint: it
// is seeded from a localStorage cache written whenever the banner is dismissed.
// A dismissed user renders nothing from the first frame (no flash, no leftover
// gap); an undismissed user renders the banner as part of the initial stable
// layout. The server-side preference stays the source of truth and reconciles
// the local cache once it resolves.
import { useEffect, useState } from "react";
import { FlaskConical, ArrowRight, X } from "lucide-react";

import { GithubIcon } from "@/components/icons/github-icon";
import { useUserPreference, useSetUserPreference } from "@/hooks/use-user-preferences";

const BANNER_DISMISSED_KEY = "ui.early_access_banner.dismissed";
// Synchronously-readable mirror of the dismissed preference, so first paint can
// decide whether to show the banner without waiting for the server round-trip.
const BANNER_DISMISSED_CACHE_KEY = "everruns:ui:early-access-banner-dismissed";
const GITHUB_URL = "https://github.com/everruns/everruns";

function readCachedDismissed(): boolean {
  if (typeof window === "undefined") return false;
  try {
    return window.localStorage.getItem(BANNER_DISMISSED_CACHE_KEY) === "true";
  } catch {
    // Storage may be unavailable (private mode, quota); treat as not dismissed.
    return false;
  }
}

function writeCachedDismissed(dismissed: boolean): void {
  if (typeof window === "undefined") return;
  try {
    if (dismissed) {
      window.localStorage.setItem(BANNER_DISMISSED_CACHE_KEY, "true");
    } else {
      window.localStorage.removeItem(BANNER_DISMISSED_CACHE_KEY);
    }
  } catch {
    // Best-effort: the server preference remains authoritative.
  }
}

export function EarlyAccessBanner() {
  const { data: preference } = useUserPreference(BANNER_DISMISSED_KEY);
  const setPreference = useSetUserPreference();
  // Seed synchronously from the local cache so the very first frame already
  // knows whether to render the banner — no post-paint insert or collapse.
  const [dismissed, setDismissed] = useState(readCachedDismissed);

  // Reconcile with the server once the preference resolves and keep the local
  // cache in sync so the next cold load on this device is correct. `undefined`
  // means the query is still loading — keep the cached decision until then.
  useEffect(() => {
    if (preference === undefined) return;
    const serverDismissed = preference?.value === true;
    setDismissed(serverDismissed);
    writeCachedDismissed(serverDismissed);
  }, [preference]);

  if (dismissed) {
    return null;
  }

  const handleDismiss = () => {
    // Hide immediately and persist locally so a reload before the write settles
    // still starts dismissed; the server write keeps cross-device state in sync.
    setDismissed(true);
    writeCachedDismissed(true);
    setPreference.mutate({ key: BANNER_DISMISSED_KEY, value: true });
  };

  return (
    <div
      data-slot="early-access-banner"
      className="flex h-9 flex-none items-center gap-2.5 border-b border-[hsl(var(--accent)/0.35)] bg-[hsl(var(--accent)/0.12)] px-3 text-primary md:h-11 md:px-[18px]"
    >
      <span className="inline-flex text-accent-foreground">
        <FlaskConical className="h-[15px] w-[15px]" />
      </span>
      <span className="text-[13px] tracking-[-0.01em]">
        <strong className="font-semibold">Early access</strong>
        <span className="hidden md:inline">
          . Everruns is still stabilizing — expect breaking changes and rough edges.
        </span>
      </span>
      <span className="flex-1" />
      <a
        href={GITHUB_URL}
        target="_blank"
        rel="noopener noreferrer"
        className="hidden items-center gap-1 whitespace-nowrap text-[13px] font-medium text-primary hover:underline hover:underline-offset-[3px] md:inline-flex"
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
