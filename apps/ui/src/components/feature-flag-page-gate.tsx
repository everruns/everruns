"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import Link from "next/link";
import type { ReactNode } from "react";
import type { FeatureFlags } from "@/lib/api/types";
import { useFeatureFlag } from "@/providers/feature-flags-provider";

export function FeatureFlagPageGate({
  flag,
  title,
  children,
}: {
  flag: keyof FeatureFlags;
  title: string;
  children: ReactNode;
}) {
  const ui = useUiTranslations();
  const enabled = useFeatureFlag(flag);

  if (!enabled) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <div className="space-y-2 text-center text-muted-foreground">
          <p className="text-lg font-medium text-foreground">
            {title} {ui("is not enabled")}
          </p>
          <p className="text-sm">
            {ui("Enable it in")}{" "}
            <Link href="/settings/features" className="font-medium text-primary hover:underline">
              {ui("Settings → Features")}
            </Link>
            .
          </p>
        </div>
      </div>
    );
  }

  return children;
}
