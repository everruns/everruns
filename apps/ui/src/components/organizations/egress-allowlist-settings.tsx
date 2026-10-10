"use client";

import { useState } from "react";
import { Loader2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { parseNetworkAccessPatterns } from "@/components/network-access-editor";
import {
  useOrgEgressAllowlist,
  useSetOrgEgressAllowlist,
  useSetOrgEgressAllowlistGrant,
} from "@/hooks/use-organizations";
import { ApiError } from "@/lib/api/client";

/**
 * Organization-level extension of the deployment's outbound allowlist.
 *
 * A platform administrator grants the org (the toggle, shown only to callers
 * the API reports `can_grant` for); org admins then keep one host pattern per
 * line. The server validates every pattern and enforces the list only while
 * the grant is on.
 */
export function EgressAllowlistSettings() {
  const { data: allowlist, isLoading, error } = useOrgEgressAllowlist();
  const savePatterns = useSetOrgEgressAllowlist();
  const saveGrant = useSetOrgEgressAllowlistGrant();
  // The unsaved edit; null shows the stored list.
  const [draft, setDraft] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const storedText = allowlist?.patterns?.join("\n") ?? "";
  const text = draft ?? storedText;

  // An unexpected payload (an older server, a proxy page) must not take the
  // whole settings page down, so anything without a pattern list renders nothing.
  if (isLoading || error || !allowlist || !Array.isArray(allowlist.patterns)) {
    // The section is optional; a load failure does not block the page.
    return null;
  }

  const editable = allowlist.granted && allowlist.can_edit;
  const dirty = text.trim() !== storedText.trim();
  const curated = allowlist.mode !== "open";

  const handleSave = async () => {
    setSaved(false);
    try {
      await savePatterns.mutateAsync(parseNetworkAccessPatterns(text));
      setDraft(null);
      setSaved(true);
    } catch {
      // Rendered from savePatterns.error below.
    }
  };

  return (
    <section aria-labelledby="organization-settings-outbound-allowlist">
      <div className="mb-3 flex items-start justify-between gap-4">
        <div>
          <h3
            id="organization-settings-outbound-allowlist"
            className="text-sm font-semibold uppercase tracking-[0.08em] text-muted-foreground"
          >
            Outbound allowlist
          </h3>
          <p className="mt-1 text-sm text-muted-foreground">
            Hosts this organization&apos;s agents, MCP servers and integrations may send data to, in
            addition to the deployment&apos;s allowlist.
          </p>
        </div>
        <Badge variant={allowlist.granted ? "accent" : "outline"}>
          {allowlist.granted ? "Enabled" : "Not enabled"}
        </Badge>
      </div>
      <Card className="gap-0 overflow-hidden p-0">
        {allowlist.can_grant && (
          <div className="flex items-center justify-between gap-4 border-b px-5 py-4">
            <div className="min-w-0">
              <Label htmlFor="egress-allowlist-grant" className="text-sm font-medium">
                Allow this organization to extend the allowlist
              </Label>
              <p className="mt-1 text-sm leading-5 text-muted-foreground">
                Platform administrators only. Turning this off keeps the list but stops enforcing
                it.
              </p>
            </div>
            <Switch
              id="egress-allowlist-grant"
              checked={allowlist.granted}
              disabled={saveGrant.isPending}
              onCheckedChange={(granted) => saveGrant.mutate(granted)}
            />
          </div>
        )}
        <div className="space-y-3 px-5 py-4">
          {!curated && (
            <p className="text-sm text-muted-foreground">
              This deployment does not restrict outbound requests, so the list has no effect here.
            </p>
          )}
          {allowlist.granted ? (
            <>
              <Label htmlFor="egress-allowlist-patterns" className="text-sm font-medium">
                Host patterns
              </Label>
              <Textarea
                id="egress-allowlist-patterns"
                value={text}
                onChange={(e) => {
                  setDraft(e.target.value);
                  setSaved(false);
                }}
                readOnly={!editable}
                rows={5}
                className="font-mono text-sm"
                placeholder={"api.example.com\n*.example.com\nhttps://example.com/mcp/"}
              />
              <p className="text-xs text-muted-foreground">
                One per line, up to {allowlist.max_patterns}: <code>example.com</code>,{" "}
                <code>*.example.com</code>, or an <code>https://example.com/path/</code> prefix.
                Public hostnames only; the deployment&apos;s deny list still applies.
              </p>
              {editable && (
                <div className="flex items-center gap-3">
                  <Button
                    size="sm"
                    onClick={handleSave}
                    disabled={!dirty || savePatterns.isPending}
                  >
                    {savePatterns.isPending && <Loader2 className="h-4 w-4 animate-spin" />}
                    Save
                  </Button>
                  {saved && !dirty && (
                    <span role="status" className="text-sm text-success">
                      Saved
                    </span>
                  )}
                </div>
              )}
            </>
          ) : (
            <p className="text-sm text-muted-foreground">
              A platform administrator must enable this before the organization can add hosts.
              {allowlist.patterns.length > 0 &&
                ` ${allowlist.patterns.length} saved pattern(s) are kept but not enforced.`}
            </p>
          )}
        </div>
      </Card>
      {(savePatterns.error || saveGrant.error) && (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {errorMessage(savePatterns.error ?? saveGrant.error)}
        </p>
      )}
    </section>
  );
}

function errorMessage(error: unknown): string {
  if (error instanceof ApiError) {
    return error.message;
  }
  return "Could not save the outbound allowlist";
}
