"use client";

import { useState } from "react";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import type { NetworkAccessList } from "@/lib/api/types";

/** Parse textarea content into a pattern list: one pattern per line, trimmed, empties dropped. */
export function parseNetworkAccessPatterns(text: string): string[] {
  return text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}

/**
 * Lightweight client-side sanity check: rejects patterns containing whitespace
 * and URL prefixes with a non-http(s) scheme. Full pattern matching semantics
 * (domain, `*.domain`, URL prefix) live server-side; anything passing here is
 * still validated by the API.
 */
export function findInvalidPatterns(patterns: string[]): string[] {
  return patterns.filter((p) => /\s/.test(p) || (p.includes("://") && !/^https?:\/\//.test(p)));
}

/**
 * Build the API value from the two textareas. An empty object (no allowed, no
 * blocked) is meaningful: the API treats `{}` as "clear restrictions on this
 * layer", while omitting the field leaves it unchanged.
 */
export function buildNetworkAccess(allowedText: string, blockedText: string): NetworkAccessList {
  const allowed = parseNetworkAccessPatterns(allowedText);
  const blocked = parseNetworkAccessPatterns(blockedText);
  const result: NetworkAccessList = {};
  if (allowed.length > 0) result.allowed = allowed;
  if (blocked.length > 0) result.blocked = blocked;
  return result;
}

/**
 * Canonical form for change detection: `null`, `{}`, and empty arrays all mean
 * "no restrictions on this layer".
 */
export function normalizeNetworkAccess(value: NetworkAccessList | null | undefined): {
  allowed: string[];
  blocked: string[];
} {
  return { allowed: value?.allowed ?? [], blocked: value?.blocked ?? [] };
}

interface NetworkAccessEditorProps {
  /** Initial value (from the agent/harness resource). */
  value: NetworkAccessList | null | undefined;
  /** Called with the parsed value on every edit. `{}` means "no restrictions". */
  onChange: (value: NetworkAccessList) => void;
  disabled?: boolean;
  description?: string;
}

/**
 * Editor for a `NetworkAccessList` (see specs/network-access.md): allowed and
 * blocked host/URL patterns, one per line. Layers (harness → agent → session)
 * can only narrow access, never widen it; blocked always wins over allowed.
 */
export function NetworkAccessEditor({
  value,
  onChange,
  disabled,
  description,
}: NetworkAccessEditorProps) {
  // Raw text is the edit-time source of truth; the parsed value flows up via
  // onChange. Initialized once from the loaded resource (pages render this
  // editor only after data is available).
  const [allowedText, setAllowedText] = useState(() => (value?.allowed ?? []).join("\n"));
  const [blockedText, setBlockedText] = useState(() => (value?.blocked ?? []).join("\n"));

  const handleAllowedChange = (text: string) => {
    setAllowedText(text);
    onChange(buildNetworkAccess(text, blockedText));
  };

  const handleBlockedChange = (text: string) => {
    setBlockedText(text);
    onChange(buildNetworkAccess(allowedText, text));
  };

  const invalidAllowed = findInvalidPatterns(parseNetworkAccessPatterns(allowedText));
  const invalidBlocked = findInvalidPatterns(parseNetworkAccessPatterns(blockedText));

  return (
    <div className="space-y-4">
      <div className="space-y-1">
        <Label>Network Access</Label>
        <p className="text-xs text-muted-foreground">
          {description ??
            "Control which hosts network-capable tools (e.g. web_fetch) can reach. One pattern per line: example.com, *.example.com, or https://example.com/api/."}
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-2">
        <div className="space-y-2">
          <Label htmlFor="network-access-allowed" className="text-xs font-medium">
            Allowed hosts
          </Label>
          <Textarea
            id="network-access-allowed"
            placeholder={"api.example.com\n*.github.com"}
            value={allowedText}
            onChange={(e) => handleAllowedChange(e.target.value)}
            disabled={disabled}
            rows={4}
            className="font-mono text-xs"
          />
          {invalidAllowed.length > 0 && (
            <p className="text-xs text-destructive">
              Invalid pattern{invalidAllowed.length > 1 ? "s" : ""}: {invalidAllowed.join(", ")}
            </p>
          )}
          <p className="text-xs text-muted-foreground">
            Leave empty to allow all hosts. When set, only matching URLs are permitted.
          </p>
        </div>

        <div className="space-y-2">
          <Label htmlFor="network-access-blocked" className="text-xs font-medium">
            Blocked hosts
          </Label>
          <Textarea
            id="network-access-blocked"
            placeholder={"internal.corp\n*.evil.com"}
            value={blockedText}
            onChange={(e) => handleBlockedChange(e.target.value)}
            disabled={disabled}
            rows={4}
            className="font-mono text-xs"
          />
          {invalidBlocked.length > 0 && (
            <p className="text-xs text-destructive">
              Invalid pattern{invalidBlocked.length > 1 ? "s" : ""}: {invalidBlocked.join(", ")}
            </p>
          )}
          <p className="text-xs text-muted-foreground">
            Always denied, even when matched by an allowed pattern.
          </p>
        </div>
      </div>
    </div>
  );
}
