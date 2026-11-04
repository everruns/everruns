"use client";

// Settings > Organization: the org's links to the shared Everruns GitHub App.
//
// Decision (D3, knowledge/integrations/github-model.md): a shared App
// installation belongs to the org, so only admins link or unlink one here.
// Agents then choose a link on their GitHub section. Unlinking removes every
// agent's choice of it; it does not uninstall the App on GitHub.

import { useLocale as useFormattingLocale, useUiTranslations } from "@/providers/locale-provider";

import { useState } from "react";
import { Loader2, Plus } from "lucide-react";
import { Button, buttonVariants } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useEverrunsAppLinks, useUnlinkEverrunsApp } from "@/hooks/use-everruns-github-app";
import { everrunsAppLinkUrl, type EverrunsAppLink } from "@/lib/api/everruns-github-app";
import { formatCountLabel } from "@/lib/formatting";
import { useOrg } from "@/providers/org-provider";

const RETURN_TO = "/settings/organization";

export function EverrunsGitHubAppSettings() {
  const { displayLocale } = useFormattingLocale();
  const ui = useUiTranslations();
  const { currentOrg } = useOrg();
  const { data: links, isLoading, error } = useEverrunsAppLinks();
  const unlink = useUnlinkEverrunsApp();
  const [pending, setPending] = useState<EverrunsAppLink | null>(null);
  const [unlinkError, setUnlinkError] = useState<string | null>(null);

  // Optional section: a load failure or a deployment without the shared App
  // leaves the rest of the page alone.
  if (isLoading || error || !links || !links.configured || !currentOrg) return null;

  const onUnlink = async () => {
    if (!pending) return;
    setUnlinkError(null);
    try {
      await unlink.mutateAsync(pending.id);
      setPending(null);
    } catch (caught) {
      setUnlinkError(caught instanceof Error ? caught.message : "Unable to unlink.");
    }
  };

  return (
    <section aria-labelledby="organization-settings-everruns-github-app">
      <div className="mb-3 flex items-start justify-between gap-4">
        <div>
          <h3
            id="organization-settings-everruns-github-app"
            className="text-sm font-semibold uppercase tracking-[0.08em] text-muted-foreground"
          >
            {ui("Everruns GitHub App")}
          </h3>
          <p className="mt-1 text-sm text-muted-foreground">
            {ui(
              "GitHub accounts where the shared Everruns App is installed for this organization. Agents can choose a linked account instead of creating their own App",
            )}
            {links.app_slug
              ? ui(", and then appear as {value0}[bot]", { value0: links.app_slug })
              : ""}
            .
          </p>
        </div>
        <a
          href={everrunsAppLinkUrl(currentOrg.public_id, RETURN_TO)}
          className={buttonVariants({ variant: "outline", size: "sm" })}
        >
          <Plus className="size-4" /> {ui("Link")}
        </a>
      </div>
      <Card className="gap-0 overflow-hidden p-0">
        {links.installations.length === 0 ? (
          <p className="px-5 py-4 text-sm text-muted-foreground">
            {ui("No GitHub account is linked yet.")}
          </p>
        ) : (
          <ul className="divide-y">
            {links.installations.map((link) => (
              <li
                key={link.id}
                className="flex flex-wrap items-center justify-between gap-3 px-5 py-3"
              >
                <div className="min-w-0">
                  <p className="break-words font-medium">{link.account}</p>
                  <p className="text-sm text-muted-foreground">
                    {[
                      link.account_type,
                      link.repository_selection === "selected"
                        ? "Selected repositories"
                        : "All repositories",
                      formatCountLabel(link.agent_count, "agent", undefined, displayLocale),
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </p>
                </div>
                <Button variant="outline" size="sm" onClick={() => setPending(link)}>
                  {ui("Unlink")}
                </Button>
              </li>
            ))}
          </ul>
        )}
      </Card>

      <Dialog open={pending !== null} onOpenChange={(open) => !open && setPending(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>
              {ui("Unlink")} {pending?.account}?
            </DialogTitle>
            <DialogDescription>
              {ui(
                "Agents using this account through the Everruns App lose their GitHub identity, and their GitHub triggers stop firing. The App stays installed on GitHub.",
              )}
            </DialogDescription>
          </DialogHeader>
          {unlinkError && <p className="text-sm text-destructive">{unlinkError}</p>}
          <DialogFooter>
            <Button variant="outline" onClick={() => setPending(null)}>
              {ui("Cancel")}
            </Button>
            <Button variant="destructive" onClick={onUnlink} disabled={unlink.isPending}>
              {unlink.isPending && <Loader2 className="size-4 animate-spin" />}
              {ui("Unlink")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
