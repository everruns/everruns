"use client";

// The Sync dialog's choice of development GitHub account.
//
// Decision (knowledge/runtime-resources/agent-sources.md): source sync is an
// authoring connection, separate from the Agent's runtime GitHub identity on
// the Integrations tab. A person syncs through their own GitHub sign-in or the
// organization's link to the shared Everruns App, and connects either one here
// in a popup without leaving the Agent page. A blocked popup falls back to a
// same-tab redirect that returns with `?sync=github`, which reopens the dialog.

import { useEffect, useRef, useState } from "react";
import { Building2, Check, GitPullRequest, Loader2, User } from "lucide-react";
import { Button } from "@/components/ui/button";
import { navigateTo } from "@/lib/browser-navigation";
import { connectErrorMessage } from "@/lib/connect-error";
import {
  organizationGitHubLinkUrl,
  personalGitHubSignInUrl,
  type AgentSourceAccounts,
  type SourceGitHubAccount,
} from "@/lib/api/agent-source";
import { cn } from "@/lib/utils";
import { useOrg } from "@/providers/org-provider";
import { useUiTranslations } from "@/providers/locale-provider";

/** The account a new Sync dialog starts on: the person's own GitHub when it is
 * connected, then an org link, then personal (the one anyone can connect). */
export function defaultSourceAccount(
  accounts: AgentSourceAccounts | undefined,
): SourceGitHubAccount {
  if (accounts?.personal.connected) return "personal";
  if (accounts?.organization.accounts.length) return "organization";
  return "personal";
}

export function SourceAccountPicker({
  agentId,
  accounts,
  value,
  disabled,
  onChange,
  onConnected,
}: {
  agentId: string;
  accounts: AgentSourceAccounts;
  value: SourceGitHubAccount;
  disabled?: boolean;
  onChange: (account: SourceGitHubAccount) => void;
  onConnected: () => void;
}) {
  const ui = useUiTranslations();
  const { currentOrg } = useOrg();
  const [waiting, setWaiting] = useState(false);
  const [connectError, setConnectError] = useState<string | null>(null);
  const cleanup = useRef<(() => void) | null>(null);
  useEffect(() => () => cleanup.current?.(), []);

  const connected =
    value === "personal" ? accounts.personal.connected : accounts.organization.accounts.length > 0;

  function connect(account: SourceGitHubAccount) {
    if (!currentOrg && account === "organization") return;
    const returnTo = `/agents/${agentId}?sync=github`;
    const url =
      account === "personal"
        ? personalGitHubSignInUrl(returnTo)
        : organizationGitHubLinkUrl(currentOrg!.public_id, returnTo);
    setConnectError(null);
    const popup = window.open(url, "everruns-github-connect", "popup,width=900,height=820");
    if (!popup) {
      navigateTo(url.replace("popup=true&", ""));
      return;
    }
    setWaiting(true);
    cleanup.current?.();
    const finish = () => {
      window.removeEventListener("message", listener);
      window.clearInterval(closed);
      cleanup.current = null;
      setWaiting(false);
    };
    const listener = (event: MessageEvent) => {
      if (event.origin !== window.location.origin) return;
      if (event.data?.type !== "everruns:connection-complete") return;
      if (event.data?.provider !== "github") return;
      finish();
      if (event.data?.status === "error") {
        setConnectError(connectErrorMessage(event.data?.connect_error));
        return;
      }
      onConnected();
    };
    // GitHub can finish without reporting back (closed window, or an error
    // page); refresh anyway so a completed connection still shows up.
    const closed = window.setInterval(() => {
      if (!popup.closed) return;
      finish();
      onConnected();
    }, 1000);
    window.addEventListener("message", listener);
    cleanup.current = finish;
  }

  const options: {
    key: SourceGitHubAccount;
    icon: typeof User;
    title: string;
    detail: string;
    status: string | null;
  }[] = [
    {
      key: "personal",
      icon: User,
      title: ui("My GitHub account"),
      detail: ui("Reads with your access. Two-way commits carry your name."),
      status: accounts.personal.connected
        ? accounts.personal.login
          ? ui("Connected as {value0}", { value0: accounts.personal.login })
          : ui("Connected")
        : null,
    },
    {
      key: "organization",
      icon: Building2,
      title: ui("Organization GitHub"),
      detail: ui("The Everruns App an admin linked. Keeps working when people leave."),
      status: accounts.organization.accounts.length
        ? accounts.organization.accounts.join(", ")
        : null,
    },
  ];

  return (
    <div className="space-y-3">
      <div role="radiogroup" aria-label={ui("Sync through")} className="grid gap-3 sm:grid-cols-2">
        {options.map((option) => {
          const selected = option.key === value;
          const Icon = option.icon;
          return (
            <button
              key={option.key}
              type="button"
              role="radio"
              aria-checked={selected}
              disabled={disabled || waiting}
              onClick={() => onChange(option.key)}
              className={cn(
                "flex items-start gap-3 border p-3 text-left transition-colors",
                selected ? "border-foreground bg-muted/50" : "hover:bg-muted/30",
              )}
            >
              <Icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
              <span className="min-w-0 space-y-1">
                <span className="block text-sm font-medium">{option.title}</span>
                <span className="block text-xs text-muted-foreground">{option.detail}</span>
                {option.status && (
                  <span className="flex items-center gap-1 break-all text-xs">
                    <Check className="size-3 shrink-0 text-success" /> {option.status}
                  </span>
                )}
              </span>
            </button>
          );
        })}
      </div>

      {!connected && (
        <div className="flex flex-wrap items-center justify-between gap-3 border p-3">
          <div className="min-w-0 space-y-1">
            {value === "organization" && !accounts.organization.can_connect ? (
              <>
                <p className="text-sm font-medium">{ui("An organization admin connects GitHub")}</p>
                <p className="text-sm text-muted-foreground">
                  {ui(
                    "Organization GitHub accounts are linked once by an admin, then every Agent can sync from them. Ask an admin, or use your own GitHub account.",
                  )}
                </p>
              </>
            ) : (
              <>
                <p className="text-sm font-medium">
                  {waiting ? ui("Waiting for GitHub…") : ui("Connect GitHub to pick a repository")}
                </p>
                <p className="text-sm text-muted-foreground">
                  {value === "personal"
                    ? ui(
                        "Sign in with GitHub in a small window. You stay on this page. It does not change how this Agent appears on GitHub.",
                      )
                    : ui(
                        "Install the Everruns App on a GitHub account or organization in a small window. You stay on this page. It does not change how this Agent appears on GitHub.",
                      )}
                </p>
              </>
            )}
          </div>
          {(value === "personal" || accounts.organization.can_connect) && (
            <Button type="button" disabled={disabled || waiting} onClick={() => connect(value)}>
              {waiting ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <GitPullRequest className="size-4" aria-hidden="true" />
              )}
              {ui("Connect GitHub")}
            </Button>
          )}
        </div>
      )}
      {connected && (
        <p className="text-xs text-muted-foreground">
          {value === "personal"
            ? ui("Missing a repository? The Everruns App must be installed on its account.")
            : ui("Missing a repository?")}{" "}
          {((value === "personal" && accounts.install_url) ||
            (value === "organization" && accounts.organization.can_connect)) && (
            <button
              type="button"
              className="underline"
              disabled={disabled || waiting}
              onClick={() =>
                value === "personal"
                  ? window.open(accounts.install_url!, "_blank", "noopener,noreferrer")
                  : connect("organization")
              }
            >
              {value === "personal"
                ? ui("Install it on GitHub")
                : ui("Connect another GitHub account")}
            </button>
          )}
        </p>
      )}
      {connectError && (
        <p role="alert" className="break-words text-sm text-destructive">
          {connectError}
        </p>
      )}
    </div>
  );
}
