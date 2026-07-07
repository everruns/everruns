import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  getUserPreference,
  setUserPreference,
  type UserPreference,
} from "@/lib/api/user-preferences";
import { queryKeys } from "@/lib/query-keys";

/**
 * Read a single user preference by key. `data` is null when the preference has
 * not been set yet.
 */
export function useUserPreference(key: string, enabled: boolean = true) {
  return useQuery({
    queryKey: queryKeys.userPreferences.detail(key),
    queryFn: () => getUserPreference(key),
    enabled,
    staleTime: 5 * 60 * 1000,
  });
}

/** Create or update a user preference value. */
export function useSetUserPreference() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({ key, value }: { key: string; value: unknown }) => setUserPreference(key, value),
    onSuccess: (preference: UserPreference) => {
      queryClient.setQueryData(queryKeys.userPreferences.detail(preference.key), preference);
      queryClient.invalidateQueries({ queryKey: queryKeys.userPreferences.list() });
    },
  });
}
