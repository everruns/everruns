"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { ApprovalDisplayText } from "./approval-display-text";
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
  const ui = useUiTranslations();
  const summary = record(value);
  const entries = Array.isArray(summary.credentials) ? summary.credentials.slice(0, 8) : [];
  return (
    <div className="mt-3 space-y-3">
      <div className="border border-border bg-background p-3">
        <p className="text-xs text-muted-foreground">{ui("Request credentials from")}</p>
        <p className="mt-1 break-all text-sm font-medium">
          <ApprovalDisplayText>{text(summary.source)}</ApprovalDisplayText>
        </p>
        <p className="mt-1 text-xs text-muted-foreground">
          <ApprovalDisplayText>{text(summary.method) || ui("GET")}</ApprovalDisplayText>{" "}
          {ui("· Acting as")}{" "}
          {text(summary.acting_as) === "user"
            ? ui("the verified user")
            : ui("the service identity")}
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
              <code className="text-xs font-medium">
                $<ApprovalDisplayText>{text(credential.name)}</ApprovalDisplayText>
              </code>
              <Badge variant="outline">
                <ApprovalDisplayText>{SCOPES[scope] ?? scope}</ApprovalDisplayText>
              </Badge>
              {credential.replacing === true && (
                <Badge variant="outline">{ui("Replaces existing value")}</Badge>
              )}
            </div>
            <p className="text-xs text-muted-foreground">
              {ui("Response field:")}{" "}
              <code>
                <ApprovalDisplayText>{text(credential.from)}</ApprovalDisplayText>
              </code>
            </p>
            <p className="text-xs text-muted-foreground">
              {ui("Stored for")}{" "}
              <span className="font-medium text-foreground">
                <ApprovalDisplayText>{text(credential.owner_label)}</ApprovalDisplayText>
              </span>
            </p>
            <div>
              <p className="text-xs text-muted-foreground">{ui("May be sent only to")}</p>
              <p className="mt-1 break-all text-sm font-medium">
                <ApprovalDisplayText>{text(binding.url)}</ApprovalDisplayText>
              </p>
            </div>
            <p className="text-xs text-muted-foreground">
              <ApprovalDisplayText>
                {auth.type === "bearer"
                  ? ui("Bearer authorization")
                  : ui("{value0}: {value1}", {
                      value0: auth.type === "form_field" ? "Form field" : "HTTP header",
                      value1: text(auth.name),
                    })}
              </ApprovalDisplayText>
            </p>
            <p className="text-xs leading-5 text-muted-foreground">
              {scope === "agent"
                ? ui("Available to authorized runs of this agent, including scheduled work.")
                : scope === "organization"
                  ? ui("Use requires an explicit organization access grant.")
                  : scope === "virtual_user"
                    ? ui(
                        "Reusable across agents when acting as this exact virtual user in this organization.",
                      )
                    : ui("Available only to the authorized acting identity in this session.")}
            </p>
            <p className="text-xs text-muted-foreground">
              {credential.expires_in_from || credential.expires_at_from
                ? ui("Expiry comes from the service response.")
                : ui("No service expiry declared. Available until revoked.")}
            </p>
          </div>
        );
      })}
      {summary.request !== undefined && (
        <details className="text-xs">
          <summary className="cursor-pointer text-muted-foreground">{ui("Exact request")}</summary>
          <pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-all border border-border p-3">
            <ApprovalDisplayText>{JSON.stringify(summary.request, null, 2)}</ApprovalDisplayText>
          </pre>
        </details>
      )}
      <p className="flex items-start gap-2 text-xs leading-5 text-muted-foreground">
        <ShieldCheck className="mt-0.5 size-3.5 shrink-0" />
        {ui(
          "Values are encrypted and hidden from the agent. This approval covers this exact capture request.",
        )}
      </p>
    </div>
  );
}
