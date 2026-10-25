"use client";

import { Badge } from "@/components/ui/badge";
import { ArrowDown, ShieldCheck } from "lucide-react";

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}
function text(value: unknown): string {
  return typeof value === "string" ? value : "";
}
const SCOPES: Record<string, string> = {
  session: "This session",
  agent: "Agent",
  virtual_user: "Virtual user",
  organization: "Organization",
};

export function CredentialConsentSummary({ value }: { value: unknown }) {
  const summary = record(value);
  const entries = Array.isArray(summary.credentials) ? summary.credentials.slice(0, 8) : [];
  return (
    <div className="mt-3 space-y-3">
      <div className="border border-border bg-background p-3">
        <p className="text-xs text-muted-foreground">Request credentials from</p>
        <p className="mt-1 break-all text-sm font-medium">{text(summary.source)}</p>
        <p className="mt-1 text-xs text-muted-foreground">
          {text(summary.method) || "GET"} · Acting as{" "}
          {text(summary.acting_as) === "user" ? "the verified user" : "the service identity"}
        </p>
      </div>
      <div className="flex items-center justify-center text-muted-foreground">
        <ArrowDown className="size-4" />
      </div>
      {entries.map((entry, index) => {
        const credential = record(entry);
        const binding = record(credential.binding);
        const auth = record(binding.auth);
        const scope = text(credential.scope);
        return (
          <div key={index} className="space-y-2 border border-border bg-background p-3">
            <div className="flex flex-wrap items-center gap-2">
              <code className="text-xs font-medium">${text(credential.name)}</code>
              <Badge variant="outline">{SCOPES[scope] ?? scope}</Badge>
              {credential.replacing === true && (
                <Badge variant="outline">Replaces existing value</Badge>
              )}
            </div>
            <p className="text-xs text-muted-foreground">
              Response field: <code>{text(credential.from)}</code>
            </p>
            <p className="text-xs text-muted-foreground">
              Stored for{" "}
              <span className="font-medium text-foreground">{text(credential.owner_label)}</span>
            </p>
            <div>
              <p className="text-xs text-muted-foreground">May be sent only to</p>
              <p className="mt-1 break-all text-sm font-medium">{text(binding.url)}</p>
            </div>
            <p className="text-xs text-muted-foreground">
              {auth.type === "bearer"
                ? "Bearer authorization"
                : `${auth.type === "form_field" ? "Form field" : "HTTP header"}: ${text(auth.name)}`}
            </p>
            <p className="text-xs leading-5 text-muted-foreground">
              {scope === "agent"
                ? "Available to authorized runs of this agent, including scheduled work."
                : scope === "organization"
                  ? "Use requires an explicit organization access grant."
                  : scope === "virtual_user"
                    ? "Reusable across agents when acting as this exact virtual user in this organization."
                    : "Available only to the authorized acting identity in this session."}
            </p>
            <p className="text-xs text-muted-foreground">
              {credential.expires_in_from || credential.expires_at_from
                ? "Expiry comes from the service response."
                : "No service expiry declared. Available until revoked."}
            </p>
          </div>
        );
      })}
      {summary.request !== undefined && (
        <details className="text-xs">
          <summary className="cursor-pointer text-muted-foreground">Exact request</summary>
          <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-all border border-border p-3">
            {JSON.stringify(summary.request, null, 2)}
          </pre>
        </details>
      )}
      <p className="flex items-start gap-2 text-xs leading-5 text-muted-foreground">
        <ShieldCheck className="mt-0.5 size-3.5 shrink-0" />
        Values are encrypted and hidden from the agent. This approval covers this exact capture
        request.
      </p>
    </div>
  );
}
