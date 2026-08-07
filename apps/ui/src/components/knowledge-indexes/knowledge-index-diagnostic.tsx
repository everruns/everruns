import { AlertCircle, CheckCircle2, Clock3, LoaderCircle } from "lucide-react";
import type { ReactNode } from "react";
import { Badge } from "@/components/ui/badge";
import { Notice, NoticeDescription, NoticeTitle } from "@/components/ui/notice";
import type { KnowledgeIndexDiagnostic } from "@/lib/knowledge-index-diagnostics";

function DiagnosticIcon({ diagnostic }: { diagnostic: KnowledgeIndexDiagnostic }) {
  if (diagnostic.code === "ready") return <CheckCircle2 aria-hidden="true" />;
  if (diagnostic.code === "queued" || diagnostic.code === "not_synced") {
    return <Clock3 aria-hidden="true" />;
  }
  if (diagnostic.code === "syncing") {
    return <LoaderCircle aria-hidden="true" className="animate-spin" />;
  }
  return <AlertCircle aria-hidden="true" />;
}

export function KnowledgeIndexDiagnosticBadge({
  diagnostic,
}: {
  diagnostic: KnowledgeIndexDiagnostic;
}) {
  return (
    <Badge
      variant={diagnostic.variant}
      role="status"
      aria-label={`Index status: ${diagnostic.label}`}
    >
      <DiagnosticIcon diagnostic={diagnostic} />
      {diagnostic.label}
    </Badge>
  );
}

export function KnowledgeIndexDiagnosticNotice({
  diagnostic,
  action,
}: {
  diagnostic: KnowledgeIndexDiagnostic;
  action?: ReactNode;
}) {
  return (
    <Notice
      variant={diagnostic.noticeVariant}
      icon={<DiagnosticIcon diagnostic={diagnostic} />}
      role={diagnostic.noticeVariant === "destructive" ? "alert" : "status"}
      aria-live="polite"
    >
      <NoticeTitle>{diagnostic.label}</NoticeTitle>
      <NoticeDescription>
        <p>{diagnostic.description}</p>
        {diagnostic.failureReason && (
          <p className="break-words text-foreground">
            <span className="font-medium">Latest failure:</span> {diagnostic.failureReason}
          </p>
        )}
        {action && <div className="pt-1">{action}</div>}
      </NoticeDescription>
    </Notice>
  );
}
