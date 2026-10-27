"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { z } from "zod";
import { usePageFormDraft } from "@/components/chat-mode/use-page-form-draft";
import { isPublicDraftLocation } from "@/components/chat-mode/page-form-schemas";
import { useState } from "react";
import { Loader2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { parseNetworkAccessPatterns } from "@/components/network-access-editor";
import { useOrgEgressAllowlist, useSetOrgEgressAllowlist } from "@/hooks/use-organizations";
import { ApiError } from "@/lib/api/client";

const publicPatterns = (text: string) =>
  text.split("\n").every((line) => !line.includes("://") || isPublicDraftLocation(line.trim()));
const allowlistDraftSchema = z.strictObject({
  host_patterns: z.string().refine(publicPatterns, "Host patterns cannot contain credentials."),
});

/**
 * Organization-level extension of the deployment's outbound allowlist.
 *
 * Org admins keep one host pattern per line. The server validates every
 * pattern and enforces the list only while the org is granted. Granting is a
 * cross-org platform decision, so it lives in the host's admin console, not
 * on the org's own settings page.
 */
export function EgressAllowlistSettings() {
  const ui = useUiTranslations();
  const { data: allowlist, isLoading, error } = useOrgEgressAllowlist();
  const savePatterns = useSetOrgEgressAllowlist();
  // The unsaved edit; null shows the stored list.
  const [draft, setDraft] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  const storedText = allowlist?.patterns?.join("\n") ?? "";
  const text = draft ?? storedText;

  const chat = usePageFormDraft({
    schema: allowlistDraftSchema,
    values: { host_patterns: text },
    entityType: "organization_network",
    label: ui("Outbound allowlist"),
    formId: "outbound_allowlist",
    enabled: !!allowlist && !isLoading && !error && publicPatterns(text),
    baseline: storedText,
    dirty: text !== storedText,
    writable: !!allowlist?.granted && !!allowlist?.can_edit && !savePatterns.isPending,
    apply: (next) => {
      setDraft(next.host_patterns);
      setSaved(false);
    },
  });

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
    if (chat.blocked) return;
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
            {ui("Outbound allowlist")}
          </h3>
          <p className="mt-1 text-sm text-muted-foreground">
            {ui(
              "Hosts this organization's agents, MCP servers and integrations may send data to, in addition to the deployment's allowlist.",
            )}
          </p>
        </div>
        <Badge variant={allowlist.granted ? "accent" : "outline"}>
          {allowlist.granted ? ui("Enabled") : ui("Not enabled")}
        </Badge>
      </div>
      {chat.notice}
      {chat.adopted && draft !== null && (
        <Button
          type="button"
          variant="outline"
          disabled={savePatterns.isPending}
          onClick={() => {
            setDraft(null);
            chat.clear();
          }}
        >
          {ui("Discard allowlist changes")}
        </Button>
      )}
      <Card className="gap-0 overflow-hidden p-0">
        <div className="space-y-3 px-5 py-4">
          {!curated && (
            <p className="text-sm text-muted-foreground">
              {ui(
                "This deployment does not restrict outbound requests, so the list has no effect here.",
              )}
            </p>
          )}
          {curated && allowlist.open && (
            <p className="text-sm text-muted-foreground">
              {ui(
                "This organization has open outbound access: its agents can reach any public host except the deployment's deny list, so the list below is not needed.",
              )}
            </p>
          )}
          {allowlist.granted ? (
            <>
              <Label htmlFor="egress-allowlist-patterns" className="text-sm font-medium">
                {ui("Host patterns")}
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
                {ui("One per line, up to")} {allowlist.max_patterns}: <code>example.com</code>,{" "}
                <code>*.example.com</code>
                {ui(", or an")} <code>https://example.com/path/</code>{" "}
                {ui("prefix. Public hostnames only; the deployment's deny list still applies.")}
              </p>
              {editable && (
                <div className="flex items-center gap-3">
                  <Button
                    size="sm"
                    onClick={handleSave}
                    disabled={chat.blocked || !dirty || savePatterns.isPending}
                  >
                    {savePatterns.isPending && <Loader2 className="h-4 w-4 animate-spin" />}
                    {ui("Save")}
                  </Button>
                  {saved && !dirty && (
                    <span role="status" className="text-sm text-success">
                      {ui("Saved")}
                    </span>
                  )}
                </div>
              )}
            </>
          ) : (
            <p className="text-sm text-muted-foreground">
              {ui(
                "A platform administrator must enable this before the organization can add hosts.",
              )}
              {allowlist.patterns.length > 0 &&
                ` ${allowlist.patterns.length} saved pattern(s) are kept but not enforced.`}
            </p>
          )}
        </div>
      </Card>
      {savePatterns.error && (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {errorMessage(savePatterns.error)}
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
