"use client";

import { TokenIcon } from "@/components/icons/facet-icons";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
import {
  usePersonalAccessTokens,
  useCreatePersonalAccessToken,
  useDeletePersonalAccessToken,
} from "@/hooks/use-auth";
import { usePageTitle } from "@/hooks";
import { useAuth } from "@/providers/auth-provider";
import { Plus, Trash2, Copy, Check, Clock, ShieldAlert } from "lucide-react";
import type {
  PersonalAccessTokenListItem,
  CreatePersonalAccessTokenRequest,
} from "@/lib/api/types";
import { EntityIdentity } from "@/components/ui/entity-identity";

// Expiration picker: one compact row of presets plus "Custom" (any day count the
// server accepts), with "never expires" split into a separate, discouraged checkbox
// so it cannot be picked by a stray click on a preset.
const TOKEN_EXPIRY_PRESETS = [
  { value: "7", label: "7 days", shortLabel: "7d", days: 7 },
  { value: "30", label: "30 days", shortLabel: "30d", days: 30 },
  { value: "90", label: "90 days", shortLabel: "90d", days: 90 },
  { value: "365", label: "1 year", shortLabel: "1y", days: 365 },
  { value: "custom", label: "Custom", shortLabel: "Custom", days: undefined },
] as const;

type TokenExpiryValue = (typeof TOKEN_EXPIRY_PRESETS)[number]["value"];

const DEFAULT_TOKEN_EXPIRY: TokenExpiryValue = "90";
// Mirrors PAT_EXPIRES_MIN_DAYS / PAT_EXPIRES_MAX_DAYS in personal_access_token_routes.rs.
const MIN_EXPIRY_DAYS = 1;
const MAX_EXPIRY_DAYS = 3650;

function parseCustomDays(raw: string): number | undefined {
  if (!/^\d+$/.test(raw.trim())) return undefined;
  const days = Number(raw);
  return days >= MIN_EXPIRY_DAYS && days <= MAX_EXPIRY_DAYS ? days : undefined;
}

function formatExpiryDate(days: number): string {
  const date = new Date(Date.now() + days * 24 * 60 * 60 * 1000);
  return date.toLocaleDateString(undefined, {
    weekday: "short",
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function PersonalAccessTokenRow({
  token,
  onDelete,
}: {
  token: PersonalAccessTokenListItem;
  onDelete: (id: string) => void;
}) {
  const formatDate = (dateStr: string | undefined) => {
    if (!dateStr) return "Never";
    return new Date(dateStr).toLocaleDateString();
  };

  return (
    <div className="flex items-center justify-between p-3 border">
      <div className="flex items-center gap-3">
        <TokenIcon className="h-5 w-5 text-muted-foreground" />
        <div>
          <div className="font-medium">
            <EntityIdentity value={token.id}>{token.name}</EntityIdentity>
          </div>
          <div className="text-sm text-muted-foreground font-mono">{token.token_prefix}</div>
        </div>
      </div>
      <div className="flex items-center gap-4">
        <div className="text-sm text-muted-foreground">
          <Clock className="h-3 w-3 inline mr-1" />
          Last used: {formatDate(token.last_used_at)}
        </div>
        {token.expires_at && (
          <Badge variant="outline" className="text-xs">
            Expires: {formatDate(token.expires_at)}
          </Badge>
        )}
        <Button
          variant="ghost"
          size="sm"
          className="text-destructive"
          onClick={() => onDelete(token.id)}
        >
          <Trash2 className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}

function CreatePersonalAccessTokenDialog({
  open,
  onOpenChange,
  onTokenCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onTokenCreated: (token: string) => void;
}) {
  const [name, setName] = useState("");
  const [expiryPreset, setExpiryPreset] = useState<TokenExpiryValue>(DEFAULT_TOKEN_EXPIRY);
  const [customDays, setCustomDays] = useState("");
  const [neverExpires, setNeverExpires] = useState(false);

  const createToken = useCreatePersonalAccessToken();
  const expiresInDays = neverExpires
    ? undefined
    : expiryPreset === "custom"
      ? parseCustomDays(customDays)
      : TOKEN_EXPIRY_PRESETS.find((preset) => preset.value === expiryPreset)?.days;
  const expiryValid = neverExpires || expiresInDays !== undefined;

  const resetForm = () => {
    setName("");
    setExpiryPreset(DEFAULT_TOKEN_EXPIRY);
    setCustomDays("");
    setNeverExpires(false);
  };

  const handleOpenChange = (nextOpen: boolean) => {
    onOpenChange(nextOpen);
    if (!nextOpen) {
      resetForm();
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!expiryValid) return;
    const data: CreatePersonalAccessTokenRequest = {
      name,
      expires_in_days: expiresInDays,
    };
    const result = await createToken.mutateAsync(data);
    onTokenCreated(result.token);
    resetForm();
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create personal access token</DialogTitle>
          <DialogDescription>
            Personal access tokens are tied to your user account, not to an organization. The token
            inherits access to every organization and resource available to your account.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="token-name">Name</Label>
            <Input
              id="token-name"
              value={name}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setName(e.target.value)}
              placeholder="My personal access token"
              required
            />
          </div>
          <div className="space-y-2">
            <Label>Expiration</Label>
            <div className="flex" role="group" aria-label="Expiration">
              {TOKEN_EXPIRY_PRESETS.map((preset, index) => {
                const selected = !neverExpires && preset.value === expiryPreset;
                return (
                  <Button
                    key={preset.value}
                    type="button"
                    variant={selected ? "default" : "outline"}
                    aria-pressed={selected}
                    aria-label={preset.label}
                    disabled={neverExpires}
                    className={`flex-1 px-2 ${index > 0 ? "-ml-px" : ""}`}
                    onClick={() => setExpiryPreset(preset.value)}
                  >
                    {preset.shortLabel}
                  </Button>
                );
              })}
            </div>
            {!neverExpires && expiryPreset === "custom" && (
              <div className="flex items-center gap-2">
                <Input
                  id="token-expiry-days"
                  type="number"
                  inputMode="numeric"
                  min={MIN_EXPIRY_DAYS}
                  max={MAX_EXPIRY_DAYS}
                  value={customDays}
                  onChange={(e: React.ChangeEvent<HTMLInputElement>) =>
                    setCustomDays(e.target.value)
                  }
                  placeholder="45"
                  className="w-24"
                  aria-label="Days until expiration"
                />
                <span className="text-sm text-muted-foreground">days</span>
              </div>
            )}
            <p className="text-xs text-muted-foreground" aria-live="polite">
              {neverExpires ? (
                "This token never expires. Use only when you own the rotation process."
              ) : expiresInDays !== undefined ? (
                <>
                  Expires on{" "}
                  <span className="font-medium text-foreground">
                    {formatExpiryDate(expiresInDays)}
                  </span>
                  .
                </>
              ) : (
                `Enter a number of days from ${MIN_EXPIRY_DAYS} to ${MAX_EXPIRY_DAYS}.`
              )}
            </p>
            <div className="flex items-center gap-2 pt-1">
              <Checkbox
                id="token-never-expires"
                checked={neverExpires}
                onCheckedChange={setNeverExpires}
              />
              <Label htmlFor="token-never-expires" className="text-sm font-normal">
                Never expires <span className="text-destructive">(not recommended)</span>
              </Label>
            </div>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => handleOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={createToken.isPending || !name || !expiryValid}>
              {createToken.isPending ? "Creating..." : "Create token"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ShowPersonalAccessTokenDialog({
  token,
  open,
  onOpenChange,
}: {
  token: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    if (token) {
      await navigator.clipboard.writeText(token);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Personal access token created</DialogTitle>
          <DialogDescription>
            Copy your token now. You won&apos;t be able to see it again!
          </DialogDescription>
        </DialogHeader>
        <div className="flex items-center gap-2">
          <div className="bg-muted p-3 font-mono text-sm flex-1 min-w-0 max-w-full whitespace-nowrap overflow-x-auto">
            {token}
          </div>
          <Button
            onClick={handleCopy}
            variant="outline"
            size="icon"
            className="shrink-0"
            aria-label={copied ? "Token copied" : "Copy token"}
            title={copied ? "Token copied" : "Copy token"}
          >
            {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
          </Button>
        </div>
        <DialogFooter>
          <Button onClick={() => onOpenChange(false)}>Done</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export default function PersonalAccessTokensPage() {
  usePageTitle("Personal access tokens", "Settings");
  const { requiresAuth } = useAuth();
  const {
    data: userTokens,
    isLoading: tokensLoading,
    error: tokensError,
  } = usePersonalAccessTokens();
  const deleteToken = useDeletePersonalAccessToken();

  const [createTokenOpen, setCreateTokenOpen] = useState(false);
  const [newToken, setNewToken] = useState<string | null>(null);

  const handleDeleteToken = async (id: string) => {
    if (
      confirm(
        "Are you sure you want to delete this personal access token? This action cannot be undone.",
      )
    ) {
      await deleteToken.mutateAsync(id);
    }
  };

  const handleTokenCreated = (token: string) => {
    setCreateTokenOpen(false);
    setNewToken(token);
  };

  // If auth is not required, show a message
  if (!requiresAuth) {
    return (
      <div className="space-y-8">
        <section>
          <div className="mb-4">
            <h2 className="text-xl font-semibold">Personal access tokens</h2>
            <p className="text-sm text-muted-foreground">
              Manage your personal access tokens for programmatic access.
            </p>
          </div>
          <Card className="p-8 text-center">
            <ShieldAlert className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">Authentication Disabled</h3>
            <p className="text-muted-foreground">
              Personal access tokens are only available when authentication is enabled. Contact your
              administrator to enable authentication.
            </p>
          </Card>
        </section>
      </div>
    );
  }

  return (
    <div className="space-y-8">
      <section>
        <div className="flex items-center justify-between mb-4">
          <div>
            <h2 className="text-xl font-semibold">Personal access tokens</h2>
            <p className="text-sm text-muted-foreground">
              Manage your personal access tokens for programmatic access. Tokens are tied to your
              user account, not to an organization.
            </p>
          </div>
          <Button onClick={() => setCreateTokenOpen(true)}>
            <Plus className="h-4 w-4 mr-2" />
            Create token
          </Button>
        </div>

        <Notice
          variant="warning"
          icon={<ShieldAlert className="h-5 w-5 shrink-0" aria-hidden="true" />}
          className="mb-4"
        >
          <NoticeTitle>Full account access</NoticeTitle>
          <NoticeDescription>
            Personal access tokens are tied to your user account (not an organization) and grant
            access to every organization and resource available to your account. Treat them like
            passwords &mdash; do not share or commit them to source control.
          </NoticeDescription>
        </Notice>

        <QueryStateWrapper
          isLoading={tokensLoading}
          error={tokensError}
          data={userTokens}
          errorMessagePrefix="Failed to load personal access tokens"
          loadingSkeleton={
            <div className="space-y-2">
              {[...Array(2)].map((_, i) => (
                <Skeleton key={i} className="h-16 w-full" />
              ))}
            </div>
          }
          emptyState={
            <Card className="p-8 text-center">
              <TokenIcon className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
              <h3 className="text-lg font-medium mb-2">No personal access tokens</h3>
              <p className="text-muted-foreground mb-4">
                Create a personal access token to access the Everruns API programmatically.
              </p>
              <Button onClick={() => setCreateTokenOpen(true)}>
                <Plus className="h-4 w-4 mr-2" />
                Create token
              </Button>
            </Card>
          }
        >
          {(items) => (
            <div className="space-y-2">
              {items.map((token) => (
                <PersonalAccessTokenRow key={token.id} token={token} onDelete={handleDeleteToken} />
              ))}
            </div>
          )}
        </QueryStateWrapper>
      </section>

      {/* Dialogs */}
      <CreatePersonalAccessTokenDialog
        open={createTokenOpen}
        onOpenChange={setCreateTokenOpen}
        onTokenCreated={handleTokenCreated}
      />
      <ShowPersonalAccessTokenDialog
        token={newToken}
        open={newToken !== null}
        onOpenChange={(open) => !open && setNewToken(null)}
      />
    </div>
  );
}
