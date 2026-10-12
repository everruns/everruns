"use client";

// The one Connect GitHub control, used by the agent's GitHub card, the GitHub
// trigger form and the template setup.
//
// Decision: where the agent's App is created is part of connecting, so the
// choice lives on the button instead of a separate always-visible field. A
// private GitHub App can only be installed on the account that owns it, and
// each agent keeps its own App (its own bot), so the menu offers the personal
// account, organizations this Everruns org already reached
// (`suggested_owners`), and a typed organization. Once the App exists, the
// button only finishes installing it.

import { useState } from "react";
import { Building2, ChevronDown, User } from "lucide-react";
import { useConnectAgentGitHub } from "@/hooks/use-agent-github";
import type { AgentGitHubStatus } from "@/lib/api/agent-github";
import { Button, buttonVariants } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPositioner,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

/** Organizations a new App could be created in. Users are left out: only the
 *  signed-in GitHub user's own account can own a personal App, and that is the
 *  "Personal account" item. Unknown kinds stay, as most are organizations. */
export function suggestedOrganizations(status: AgentGitHubStatus | undefined): string[] {
  return (status?.suggested_owners ?? [])
    .filter((owner) => owner.kind !== "user")
    .map((owner) => owner.login);
}

export function ConnectGitHubButton({
  agentId,
  status,
  returnTo,
  size = "sm",
  variant = "outline",
}: {
  agentId: string;
  status: AgentGitHubStatus | undefined;
  /** Where GitHub sends the browser back; defaults to the agent's Integrations tab. */
  returnTo?: string;
  size?: "sm" | "default";
  variant?: "outline" | "default";
}) {
  const connect = useConnectAgentGitHub(agentId, returnTo);
  // "Another organization…" swaps the button for a small input; null hides it.
  const [otherOrg, setOtherOrg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const organizations = suggestedOrganizations(status);

  const onConnect = async (ownerOrg?: string) => {
    setError(null);
    try {
      await connect.mutateAsync(ownerOrg?.trim() || undefined);
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : "Unable to start GitHub connection.");
    }
  };

  const errorText = error && <p className="text-sm text-destructive">{error}</p>;

  if (status?.app_created) {
    return (
      <>
        <Button
          type="button"
          variant={variant}
          size={size}
          onClick={() => onConnect()}
          disabled={connect.isPending}
        >
          Finish installing
        </Button>
        {errorText}
      </>
    );
  }

  if (otherOrg !== null) {
    const inputId = `github-owner-${agentId}`;
    const submit = () => {
      if (otherOrg.trim()) void onConnect(otherOrg);
    };
    // Not a <form>: the trigger editor renders this inside its own form.
    return (
      <div className="space-y-1">
        <Label htmlFor={inputId}>GitHub organization</Label>
        <div className="flex flex-wrap items-center gap-2">
          <Input
            id={inputId}
            value={otherOrg}
            onChange={(event) => setOtherOrg(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                submit();
              }
            }}
            placeholder="your-org"
            className="max-w-xs"
          />
          <Button
            type="button"
            size={size}
            onClick={submit}
            disabled={connect.isPending || !otherOrg.trim()}
          >
            Connect
          </Button>
          <Button type="button" variant="ghost" size={size} onClick={() => setOtherOrg(null)}>
            Cancel
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          You need permission to create GitHub Apps in that organization.
        </p>
        {errorText}
      </div>
    );
  }

  return (
    <>
      <DropdownMenu>
        <DropdownMenuTrigger
          className={buttonVariants({ variant, size })}
          disabled={connect.isPending}
        >
          Connect GitHub <ChevronDown className="size-4" aria-hidden="true" />
        </DropdownMenuTrigger>
        <DropdownMenuPositioner align="start">
          <DropdownMenuContent className="w-60">
            <DropdownMenuGroup>
              <DropdownMenuLabel>Create the App in</DropdownMenuLabel>
              <DropdownMenuItem onClick={() => onConnect()}>
                <User className="size-4" aria-hidden="true" />
                Personal account
              </DropdownMenuItem>
              {organizations.map((login) => (
                <DropdownMenuItem key={login} onClick={() => onConnect(login)}>
                  <Building2 className="size-4" aria-hidden="true" />
                  <span className="truncate">{login}</span>
                </DropdownMenuItem>
              ))}
            </DropdownMenuGroup>
            <DropdownMenuSeparator />
            <DropdownMenuItem onClick={() => setOtherOrg("")}>
              Another organization…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenuPositioner>
      </DropdownMenu>
      {errorText}
    </>
  );
}
