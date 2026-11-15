"use client";

// Announces a switch the app made by itself to open a page in the org that owns
// it (a link to another of the user's orgs), with a way back. A silent switch
// made another org's data look like a cross-tenant leak.
// See knowledge/security/multitenancy.md (Cross-Org Resource Resolution).

import { useEffect, useRef } from "react";
import { useOrg } from "@/providers/org-provider";
import { useUiTranslations } from "@/providers/locale-provider";
import type { LocalToastInput } from "@/providers/notifications-provider";

export function OrgSwitchNotice({ notify }: { notify: (input: LocalToastInput) => void }) {
  const { autoSwitch, setCurrentOrg } = useOrg();
  const ui = useUiTranslations();
  const announcedRef = useRef(0);

  useEffect(() => {
    if (!autoSwitch || autoSwitch.id <= announcedRef.current) return;
    announcedRef.current = autoSwitch.id;
    const { from, to } = autoSwitch;
    notify({
      title: ui("Switched to {org}", { org: to.name }),
      body: ui("This page belongs to {org}, so the app switched organizations to open it.", {
        org: to.name,
      }),
      // Going back leaves the page: it does not exist in the other org.
      action: from
        ? { label: ui("Back to {org}", { org: from.name }), onClick: () => setCurrentOrg(from) }
        : undefined,
    });
  }, [autoSwitch, notify, setCurrentOrg, ui]);

  return null;
}
