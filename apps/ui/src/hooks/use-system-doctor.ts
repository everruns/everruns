// System doctor hooks. The server caches a report for a minute, so polling
// once a minute shows what any other viewer sees without running the checks
// again.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getSystemDoctor } from "@/lib/api/system";
import type { DoctorReport } from "@/lib/api/types";

const SYSTEM_DOCTOR_KEY = ["system", "doctor"] as const;

export function useSystemDoctor(options: { enabled?: boolean } = {}) {
  return useQuery({
    queryKey: SYSTEM_DOCTOR_KEY,
    queryFn: () => getSystemDoctor(false),
    refetchInterval: 60_000,
    retry: false,
    enabled: options.enabled ?? true,
  });
}

/** Run every check now and show the fresh report. */
export function useRefreshSystemDoctor() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => getSystemDoctor(true),
    onSuccess: (report: DoctorReport) => {
      queryClient.setQueryData(SYSTEM_DOCTOR_KEY, report);
    },
  });
}
