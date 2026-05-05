"use client";

import { useEffect } from "react";
import { formatPageTitle } from "@/lib/page-title";

// Sets document.title to a formatted page title. Pass null/undefined for
// segments that are still loading; once they resolve, the effect re-runs
// with the final title. See specs/code-organization.md (Page Titles).
export function usePageTitle(...parts: Array<string | null | undefined>): void {
  const title = formatPageTitle(...parts);

  useEffect(() => {
    if (typeof document === "undefined") return;
    const previous = document.title;
    document.title = title;
    return () => {
      document.title = previous;
    };
  }, [title]);
}
