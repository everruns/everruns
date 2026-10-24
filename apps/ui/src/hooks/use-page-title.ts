"use client";

import { useEffect } from "react";
import { formatPageTitle } from "@/lib/page-title";
import { usePageChatMode } from "@/components/chat-mode/chat-mode-context";
import { pageLocation } from "@/lib/chat-page-view";

// Sets document.title to a formatted page title. Pass null/undefined for
// segments that are still loading; once they resolve, the effect re-runs
// with the final title. See knowledge/foundations/code-organization.md (Page Titles).
export function usePageTitle(...parts: Array<string | null | undefined>): void {
  const title = formatPageTitle(...parts);
  const mode = usePageChatMode();
  const register = mode?.registerPage;
  const route = mode?.page.route;
  const label = parts.filter(Boolean).join(" · ").slice(0, 180);
  const specificity = Math.min(parts.filter(Boolean).length, 5);
  useEffect(() => {
    if (register && route && label)
      return register(pageLocation(route, label), undefined, specificity);
  }, [register, route, label, specificity]);

  useEffect(() => {
    if (typeof document === "undefined") return;
    const previous = document.title;
    document.title = title;
    return () => {
      document.title = previous;
    };
  }, [title]);
}
