// System doctor API (platform users only).

import { api } from "./client";
import type { DoctorReport } from "./types";

/** The install's health report; `refresh` runs every check now. */
export async function getSystemDoctor(refresh = false): Promise<DoctorReport> {
  const response = await api.get<DoctorReport>(
    refresh ? "/v1/system/doctor?refresh=true" : "/v1/system/doctor",
  );
  return response.data;
}
