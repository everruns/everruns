import { api } from "./client";
import type { TraceScore } from "./observer-types";

export interface QualityMonitoringPolicy {
  enabled: boolean;
  sampling_rate: number;
  model_id: string | null;
  daily_budget_usd: number;
  excluded_tags: string[];
}

export interface QualityMonitoringReport {
  policy: QualityMonitoringPolicy;
  observer_id: string | null;
  today: { reserved_usd: number; reported_usd: number };
  checks: {
    check: string;
    passed: number;
    failed: number;
    insufficient_evidence: number;
    pending: number;
  }[];
}

declare module "./organization-types" {
  interface Organization {
    quality_monitoring?: QualityMonitoringPolicy;
  }
  interface UpdateOrganizationRequest {
    quality_monitoring?: QualityMonitoringPolicy;
  }
}
declare module "./legacy-api-types" {
  interface Organization {
    quality_monitoring?: QualityMonitoringPolicy;
  }
  interface UpdateOrganizationRequest {
    quality_monitoring?: QualityMonitoringPolicy;
  }
}

export async function getQualityMonitoring(org: string): Promise<QualityMonitoringReport> {
  return (
    await api.get<QualityMonitoringReport>(`/v1/orgs/${encodeURIComponent(org)}/quality-monitoring`)
  ).data;
}

export async function getQualityMonitoringScores(org: string): Promise<TraceScore[]> {
  return (
    await api.get<TraceScore[]>(`/v1/orgs/${encodeURIComponent(org)}/quality-monitoring/scores`)
  ).data;
}

export async function reviewQualityMonitoringScore(
  org: string,
  score: string,
  agrees: boolean,
): Promise<TraceScore> {
  return (
    await api.put<TraceScore>(
      `/v1/orgs/${encodeURIComponent(org)}/quality-monitoring/scores/${encodeURIComponent(score)}/review`,
      { agrees },
    )
  ).data;
}
