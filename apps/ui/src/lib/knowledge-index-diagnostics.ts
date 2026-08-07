import type { KnowledgeIndex, ModelWithProvider } from "@/lib/api/types";

export type KnowledgeIndexDiagnosticCode =
  | "configuration_invalid"
  | "provider_unavailable"
  | "sync_failed"
  | "queued"
  | "syncing"
  | "not_synced"
  | "empty"
  | "stale"
  | "ready";

export type KnowledgeIndexDiagnosticAction =
  | "configure_model"
  | "configure_provider"
  | "retry_sync"
  | "start_sync"
  | "sync_changes"
  | "sync_again";

export interface KnowledgeIndexDiagnostic {
  code: KnowledgeIndexDiagnosticCode;
  label: string;
  description: string;
  variant: "secondary" | "destructive" | "accent" | "success";
  noticeVariant: "info" | "warning" | "destructive" | "success";
  action?: KnowledgeIndexDiagnosticAction;
  providerId?: string;
  failureReason?: string;
}

interface DiagnosticContext {
  models?: ModelWithProvider[];
  modelsReady?: boolean;
}

export function knowledgeIndexDiagnosticBlocksSync(diagnostic: KnowledgeIndexDiagnostic): boolean {
  return diagnostic.code === "configuration_invalid" || diagnostic.code === "provider_unavailable";
}

export function knowledgeIndexSyncActionLabel(diagnostic: KnowledgeIndexDiagnostic): string {
  switch (diagnostic.action) {
    case "start_sync":
      return "Start initial sync";
    case "retry_sync":
      return "Retry sync";
    case "sync_changes":
      return "Sync changes";
    case "sync_again":
      return "Sync again";
    default:
      return "Sync now";
  }
}

function didConfigurationChangeAfterSync(index: KnowledgeIndex): boolean {
  if (!index.last_synced_at) return false;
  const syncedAt = Date.parse(index.last_synced_at);
  const updatedAt = Date.parse(index.updated_at);
  return Number.isFinite(syncedAt) && Number.isFinite(updatedAt) && updatedAt > syncedAt + 1000;
}

function withFailureReason(
  diagnostic: KnowledgeIndexDiagnostic,
  index: KnowledgeIndex,
): KnowledgeIndexDiagnostic {
  return index.last_sync_error
    ? { ...diagnostic, failureReason: index.last_sync_error }
    : diagnostic;
}

export function getKnowledgeIndexDiagnostic(
  index: KnowledgeIndex,
  context: DiagnosticContext = {},
): KnowledgeIndexDiagnostic {
  const model = context.models?.find((candidate) => candidate.id === index.embedding_model_id);

  if (context.modelsReady && !model) {
    return withFailureReason(
      {
        code: "configuration_invalid",
        label: "Embedding model unavailable",
        description: "Choose an available embedding model before syncing.",
        variant: "destructive",
        noticeVariant: "destructive",
        action: "configure_model",
      },
      index,
    );
  }

  if (
    model &&
    !model.capabilities.some((capability) => capability.toLowerCase() === "embeddings")
  ) {
    return withFailureReason(
      {
        code: "configuration_invalid",
        label: "Embedding model invalid",
        description: `${model.display_name} does not support embeddings.`,
        variant: "destructive",
        noticeVariant: "destructive",
        action: "configure_model",
      },
      index,
    );
  }

  if (model && !model.enabled) {
    return withFailureReason(
      {
        code: "configuration_invalid",
        label: "Embedding model disabled",
        description: `${model.display_name} is disabled. Choose an enabled embedding model.`,
        variant: "destructive",
        noticeVariant: "destructive",
        action: "configure_model",
      },
      index,
    );
  }

  if (model && !model.healthy) {
    return withFailureReason(
      {
        code: "provider_unavailable",
        label: "Provider setup needed",
        description: `${model.provider_name} is not configured or available for embeddings.`,
        variant: "destructive",
        noticeVariant: "destructive",
        action: "configure_provider",
        providerId: model.provider_id,
      },
      index,
    );
  }

  if (index.sync_status === "failed") {
    return withFailureReason(
      {
        code: "sync_failed",
        label: "Sync failed",
        description: "The latest sync did not complete.",
        variant: "destructive",
        noticeVariant: "destructive",
        action: "retry_sync",
      },
      index,
    );
  }

  if (index.sync_status === "pending") {
    return {
      code: "queued",
      label: "Sync queued",
      description: "Waiting for a sync worker.",
      variant: "accent",
      noticeVariant: "info",
    };
  }

  if (index.sync_status === "syncing") {
    return {
      code: "syncing",
      label: "Syncing",
      description: "Documents are being indexed now.",
      variant: "accent",
      noticeVariant: "info",
    };
  }

  if (!index.last_synced_at) {
    return {
      code: "not_synced",
      label: "Not synced yet",
      description: "Start the initial sync to index documents.",
      variant: "secondary",
      noticeVariant: "warning",
      action: "start_sync",
    };
  }

  if (didConfigurationChangeAfterSync(index)) {
    return {
      code: "stale",
      label: "Sync out of date",
      description: "Index configuration changed after the last successful sync.",
      variant: "accent",
      noticeVariant: "warning",
      action: "sync_changes",
    };
  }

  if (index.sync_status === "synced" && index.document_count === 0) {
    return {
      code: "empty",
      label: "Synced — no documents",
      description: "The last sync succeeded but found no indexable documents.",
      variant: "accent",
      noticeVariant: "warning",
      action: "sync_again",
    };
  }

  return {
    code: "ready",
    label: "Up to date",
    description: `${index.document_count} ${index.document_count === 1 ? "document" : "documents"} indexed.`,
    variant: "success",
    noticeVariant: "success",
  };
}
