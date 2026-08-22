import type { OutputMessageCompletedData, TurnFailedData } from "@/lib/api/types";
import { getTextFromContent, isRecord } from "@/lib/api/types";
import { formatMessage, type SupportedLocale } from "@/lib/i18n";

export interface RuntimeErrorInfo {
  code: string;
  fields?: Record<string, unknown>;
}

export function getRuntimeErrorFromOutputMessage(
  data?: OutputMessageCompletedData | null,
): RuntimeErrorInfo | undefined {
  if (!data) return undefined;

  if (typeof data.error_code === "string") {
    return {
      code: data.error_code,
      fields: isRecord(data.error_fields) ? data.error_fields : undefined,
    };
  }

  const metadata = isRecord(data.message?.metadata) ? data.message.metadata : undefined;
  if (!metadata || typeof metadata.error_code !== "string") {
    return undefined;
  }

  return {
    code: metadata.error_code,
    fields: isRecord(metadata.error_fields) ? metadata.error_fields : undefined,
  };
}

export function getRuntimeErrorFromTurnFailed(
  data?: TurnFailedData | null,
): RuntimeErrorInfo | undefined {
  if (!data?.error_code) return undefined;
  return {
    code: data.error_code,
    fields: isRecord(data.error_fields) ? data.error_fields : undefined,
  };
}

export function getLocalizedOutputMessageText(
  locale: SupportedLocale,
  data: OutputMessageCompletedData,
): string {
  const fallback = getTextFromContent(data.message?.content ?? []);
  return localizeRuntimeError(locale, getRuntimeErrorFromOutputMessage(data), fallback);
}

export function localizeRuntimeError(
  locale: SupportedLocale,
  error: RuntimeErrorInfo | undefined,
  fallback: string,
): string {
  if (!error?.code) return fallback;

  const localized = localizeRuntimeErrorBase(locale, error, fallback);
  if (localized === fallback) return localized;

  // Detailed disclosure mode attaches the underlying provider error as a
  // `detail` field; the backend fallback text already includes it, so append
  // it only when the text was replaced with a localized template.
  const detail = stringField(error.fields, "detail");
  if (!detail) return localized;
  return `${localized}\n\n${formatMessage(locale, "runtime_error_details_label")}: ${detail}`;
}

function localizeRuntimeErrorBase(
  locale: SupportedLocale,
  error: RuntimeErrorInfo,
  fallback: string,
): string {
  switch (error.code) {
    case "budget_exhausted":
      return localizeBudgetExhausted(locale, error.fields, fallback);
    case "budget_paused":
      return localizeBudgetPaused(locale, error.fields, fallback);
    case "model_unavailable": {
      const modelId = stringField(error.fields, "model_id");
      return modelId
        ? formatMessage(locale, "runtime_error_model_unavailable", { value: modelId })
        : formatMessage(locale, "runtime_error_model_unavailable_generic");
    }
    case "model_not_configured":
      return formatMessage(locale, "runtime_error_model_not_configured");
    case "request_too_large":
      return formatMessage(locale, "runtime_error_request_too_large");
    case "provider_rate_limited": {
      const retryAfter = integerField(error.fields, "retry_after");
      return retryAfter != null
        ? formatMessage(locale, "runtime_error_provider_rate_limited_retry_after", {
            retry_after: retryAfter,
          })
        : formatMessage(locale, "runtime_error_provider_rate_limited");
    }
    case "provider_misconfigured":
      return formatMessage(locale, "runtime_error_provider_misconfigured");
    case "provider_quota_exhausted":
      return formatMessage(locale, "runtime_error_provider_quota_exhausted");
    case "provider_unavailable":
      return formatMessage(locale, "runtime_error_provider_unavailable");
    case "processing_error":
      return formatMessage(locale, "runtime_error_processing_error");
    case "dependency_unavailable":
      return formatMessage(locale, "runtime_error_dependency_unavailable");
    case "invalid_tool_schema":
      return formatMessage(locale, "runtime_error_invalid_tool_schema");
    default:
      return fallback;
  }
}

function localizeBudgetExhausted(
  locale: SupportedLocale,
  fields: Record<string, unknown> | undefined,
  fallback: string,
): string {
  const spent = numberField(fields, "spent");
  const limit = numberField(fields, "limit");
  const currency = stringField(fields, "currency");
  if (spent == null || limit == null || !currency) {
    return fallback || formatMessage(locale, "runtime_error_budget_exhausted_generic");
  }

  return formatMessage(
    locale,
    spent > limit
      ? "runtime_error_budget_exhausted_exceeded"
      : "runtime_error_budget_exhausted_reached",
    {
      spent: formatFixed(locale, spent),
      limit: formatFixed(locale, limit),
      currency,
    },
  );
}

function localizeBudgetPaused(
  locale: SupportedLocale,
  fields: Record<string, unknown> | undefined,
  fallback: string,
): string {
  const spent = numberField(fields, "spent");
  const softLimit = numberField(fields, "soft_limit");
  const currency = stringField(fields, "currency");

  if (spent == null || !currency) {
    return fallback || formatMessage(locale, "runtime_error_budget_paused_generic");
  }

  if (softLimit == null) {
    return formatMessage(locale, "runtime_error_budget_paused_with_spent", {
      spent: formatFixed(locale, spent),
      currency,
    });
  }

  return formatMessage(
    locale,
    spent > softLimit
      ? "runtime_error_budget_paused_exceeded"
      : "runtime_error_budget_paused_reached",
    {
      spent: formatFixed(locale, spent),
      soft_limit: formatFixed(locale, softLimit),
      currency,
    },
  );
}

function formatFixed(locale: SupportedLocale, value: number): string {
  return new Intl.NumberFormat(locale === "uk" ? "uk-UA" : "en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(value);
}

function stringField(fields: Record<string, unknown> | undefined, key: string): string | undefined {
  return typeof fields?.[key] === "string" ? (fields[key] as string) : undefined;
}

function numberField(fields: Record<string, unknown> | undefined, key: string): number | undefined {
  const value = fields?.[key];
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return undefined;
}

function integerField(
  fields: Record<string, unknown> | undefined,
  key: string,
): number | undefined {
  const value = numberField(fields, key);
  return value == null ? undefined : Math.trunc(value);
}
