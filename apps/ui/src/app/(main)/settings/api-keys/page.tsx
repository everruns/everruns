"use client";

import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Skeleton } from "@/components/ui/skeleton";
import { QueryStateWrapper } from "@/components/query-state-wrapper";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
import { useApiKeys, useCreateApiKey, useDeleteApiKey } from "@/hooks/use-auth";
import { useAuth } from "@/providers/auth-provider";
import { Plus, Key, Trash2, Copy, Check, Clock, ShieldAlert } from "lucide-react";
import type { ApiKeyListItem, CreateApiKeyRequest } from "@/lib/api/types";

const API_KEY_EXPIRY_OPTIONS = [
  {
    value: "1",
    label: "1 day",
    expiresInDays: 1,
    description: "Short-lived key for temporary use.",
  },
  {
    value: "30",
    label: "30 days",
    expiresInDays: 30,
    description: "Reasonable default for ongoing integration work.",
  },
  {
    value: "90",
    label: "90 days",
    expiresInDays: 90,
    description: "Recommended balance between longevity and rotation.",
  },
  {
    value: "unrestricted",
    label: "Unrestricted (not recommended)",
    expiresInDays: undefined,
    description: "This key never expires. Use only when you own the rotation process.",
  },
] as const;

type ApiKeyExpiryOption = (typeof API_KEY_EXPIRY_OPTIONS)[number];
type ApiKeyExpiryValue = ApiKeyExpiryOption["value"];

const DEFAULT_API_KEY_EXPIRY: ApiKeyExpiryValue = "90";

function ApiKeyRow({
  apiKey,
  onDelete,
}: {
  apiKey: ApiKeyListItem;
  onDelete: (id: string) => void;
}) {
  const formatDate = (dateStr: string | undefined) => {
    if (!dateStr) return "Never";
    return new Date(dateStr).toLocaleDateString();
  };

  return (
    <div className="flex items-center justify-between p-3 border">
      <div className="flex items-center gap-3">
        <Key className="h-5 w-5 text-muted-foreground" />
        <div>
          <div className="font-medium">{apiKey.name}</div>
          <div className="text-sm text-muted-foreground font-mono">{apiKey.key_prefix}...</div>
        </div>
      </div>
      <div className="flex items-center gap-4">
        <div className="text-sm text-muted-foreground">
          <Clock className="h-3 w-3 inline mr-1" />
          Last used: {formatDate(apiKey.last_used_at)}
        </div>
        {apiKey.expires_at && (
          <Badge variant="outline" className="text-xs">
            Expires: {formatDate(apiKey.expires_at)}
          </Badge>
        )}
        <Button
          variant="ghost"
          size="sm"
          className="text-destructive"
          onClick={() => onDelete(apiKey.id)}
        >
          <Trash2 className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}

function CreateApiKeyDialog({
  open,
  onOpenChange,
  onKeyCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onKeyCreated: (key: string) => void;
}) {
  const [name, setName] = useState("");
  const [expiryPreset, setExpiryPreset] = useState<ApiKeyExpiryValue>(DEFAULT_API_KEY_EXPIRY);

  const createApiKey = useCreateApiKey();
  const selectedExpiryOption =
    API_KEY_EXPIRY_OPTIONS.find((option) => option.value === expiryPreset) ??
    API_KEY_EXPIRY_OPTIONS[2];

  const resetForm = () => {
    setName("");
    setExpiryPreset(DEFAULT_API_KEY_EXPIRY);
  };

  const handleOpenChange = (nextOpen: boolean) => {
    onOpenChange(nextOpen);
    if (!nextOpen) {
      resetForm();
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const data: CreateApiKeyRequest = {
      name,
      expires_in_days: selectedExpiryOption.expiresInDays,
    };
    const result = await createApiKey.mutateAsync(data);
    onKeyCreated(result.key);
    resetForm();
  };

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create API Key</DialogTitle>
          <DialogDescription>
            Create a personal API key for programmatic access. This key will have access to all
            organizations and resources available to your account.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="key-name">Name</Label>
            <Input
              id="key-name"
              value={name}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => setName(e.target.value)}
              placeholder="My API Key"
              required
            />
          </div>
          <div className="space-y-2">
            <Label>Expiration</Label>
            <div className="grid grid-cols-2 gap-2" role="group" aria-label="Expiration">
              {API_KEY_EXPIRY_OPTIONS.map((option) => {
                const selected = option.value === expiryPreset;
                return (
                  <Button
                    key={option.value}
                    type="button"
                    variant={selected ? "default" : "outline"}
                    aria-pressed={selected}
                    className="h-auto justify-start whitespace-normal px-3 py-2 text-left"
                    onClick={() => setExpiryPreset(option.value)}
                  >
                    {option.label}
                  </Button>
                );
              })}
            </div>
            <p className="text-xs text-muted-foreground">{selectedExpiryOption.description}</p>
          </div>
          <DialogFooter>
            <Button type="button" variant="outline" onClick={() => handleOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={createApiKey.isPending || !name}>
              {createApiKey.isPending ? "Creating..." : "Create API Key"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function ShowApiKeyDialog({
  apiKey,
  open,
  onOpenChange,
}: {
  apiKey: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    if (apiKey) {
      await navigator.clipboard.writeText(apiKey);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>API Key Created</DialogTitle>
          <DialogDescription>
            Copy your API key now. You won&apos;t be able to see it again!
          </DialogDescription>
        </DialogHeader>
        <div className="flex items-center gap-2">
          <div className="bg-muted p-3 font-mono text-sm flex-1 min-w-0 max-w-full whitespace-nowrap overflow-x-auto">
            {apiKey}
          </div>
          <Button
            onClick={handleCopy}
            variant="outline"
            size="icon"
            className="shrink-0"
            aria-label={copied ? "API key copied" : "Copy API key"}
            title={copied ? "API key copied" : "Copy API key"}
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

export default function ApiKeysPage() {
  const { requiresAuth } = useAuth();
  const { data: userApiKeys, isLoading: apiKeysLoading, error: apiKeysError } = useApiKeys();
  const deleteApiKey = useDeleteApiKey();

  const [createApiKeyOpen, setCreateApiKeyOpen] = useState(false);
  const [newApiKey, setNewApiKey] = useState<string | null>(null);

  const handleDeleteApiKey = async (id: string) => {
    if (confirm("Are you sure you want to delete this API key? This action cannot be undone.")) {
      await deleteApiKey.mutateAsync(id);
    }
  };

  const handleApiKeyCreated = (key: string) => {
    setCreateApiKeyOpen(false);
    setNewApiKey(key);
  };

  // If auth is not required, show a message
  if (!requiresAuth) {
    return (
      <div className="space-y-8">
        <section>
          <div className="mb-4">
            <h2 className="text-xl font-semibold">API Keys</h2>
            <p className="text-sm text-muted-foreground">
              Manage your API keys for programmatic access.
            </p>
          </div>
          <Card className="p-8 text-center">
            <ShieldAlert className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
            <h3 className="text-lg font-medium mb-2">Authentication Disabled</h3>
            <p className="text-muted-foreground">
              API keys are only available when authentication is enabled. Contact your administrator
              to enable authentication.
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
            <h2 className="text-xl font-semibold">API Keys</h2>
            <p className="text-sm text-muted-foreground">
              Manage your personal API keys for programmatic access.
            </p>
          </div>
          <Button onClick={() => setCreateApiKeyOpen(true)}>
            <Plus className="h-4 w-4 mr-2" />
            Create API Key
          </Button>
        </div>

        <Notice
          variant="warning"
          icon={<ShieldAlert className="h-5 w-5 shrink-0" aria-hidden="true" />}
          className="mb-4"
        >
          <NoticeTitle>Full account access</NoticeTitle>
          <NoticeDescription>
            API keys grant access to all organizations and resources available to your account.
            Treat them like passwords &mdash; do not share or commit them to source control.
          </NoticeDescription>
        </Notice>

        <QueryStateWrapper
          isLoading={apiKeysLoading}
          error={apiKeysError}
          data={userApiKeys}
          errorMessagePrefix="Failed to load API keys"
          loadingSkeleton={
            <div className="space-y-2">
              {[...Array(2)].map((_, i) => (
                <Skeleton key={i} className="h-16 w-full" />
              ))}
            </div>
          }
          emptyState={
            <Card className="p-8 text-center">
              <Key className="h-12 w-12 mx-auto text-muted-foreground mb-4" />
              <h3 className="text-lg font-medium mb-2">No API keys</h3>
              <p className="text-muted-foreground mb-4">
                Create an API key to access the Everruns API programmatically.
              </p>
              <Button onClick={() => setCreateApiKeyOpen(true)}>
                <Plus className="h-4 w-4 mr-2" />
                Create API Key
              </Button>
            </Card>
          }
        >
          {(items) => (
            <div className="space-y-2">
              {items.map((apiKey) => (
                <ApiKeyRow key={apiKey.id} apiKey={apiKey} onDelete={handleDeleteApiKey} />
              ))}
            </div>
          )}
        </QueryStateWrapper>
      </section>

      {/* Dialogs */}
      <CreateApiKeyDialog
        open={createApiKeyOpen}
        onOpenChange={setCreateApiKeyOpen}
        onKeyCreated={handleApiKeyCreated}
      />
      <ShowApiKeyDialog
        apiKey={newApiKey}
        open={newApiKey !== null}
        onOpenChange={(open) => !open && setNewApiKey(null)}
      />
    </div>
  );
}
