// Session Schedules API functions
// Org is sent via everruns_org cookie (set by OrgProvider via /v1/users/me/switch-org)

import { api } from "./client";
import type { SessionSchedule, UpdateSessionScheduleRequest } from "./types";

/** List all schedules for a session */
export async function listSessionSchedules(sessionId: string): Promise<SessionSchedule[]> {
  const response = await api.get<SessionSchedule[]>(`/v1/sessions/${sessionId}/schedules`);
  return response.data;
}

/** Get a specific schedule */
export async function getSessionSchedule(
  sessionId: string,
  scheduleId: string,
): Promise<SessionSchedule> {
  const response = await api.get<SessionSchedule>(
    `/v1/sessions/${sessionId}/schedules/${scheduleId}`,
  );
  return response.data;
}

/** Update a schedule (enable/disable) */
export async function updateSessionSchedule(
  sessionId: string,
  scheduleId: string,
  request: UpdateSessionScheduleRequest,
): Promise<SessionSchedule> {
  const response = await api.patch<SessionSchedule>(
    `/v1/sessions/${sessionId}/schedules/${scheduleId}`,
    request,
  );
  return response.data;
}

/** Delete a schedule */
export async function deleteSessionSchedule(sessionId: string, scheduleId: string): Promise<void> {
  await api.delete(`/v1/sessions/${sessionId}/schedules/${scheduleId}`);
}

/** Manually trigger a schedule */
export async function triggerSessionSchedule(
  sessionId: string,
  scheduleId: string,
): Promise<SessionSchedule> {
  const response = await api.post<SessionSchedule>(
    `/v1/sessions/${sessionId}/schedules/${scheduleId}/trigger`,
  );
  return response.data;
}
