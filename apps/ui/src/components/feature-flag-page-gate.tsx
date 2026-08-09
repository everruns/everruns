"use client";

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
  const enabled = useFeatureFlag(flag);

  if (!enabled) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <div className="space-y-2 text-center text-muted-foreground">
          <p className="text-lg font-medium text-foreground">{title} is not enabled</p>
          <p className="text-sm">
            Enable it in{" "}
            <Link href="/settings/features" className="font-medium text-primary hover:underline">
              Settings → Features
            </Link>
            .
          </p>
        </div>
      </div>
    );
  }

  return children;
}
