// React Query hooks for user management
import { useQuery } from "@tanstack/react-query";
import { listUsers } from "@/lib/api/users";
import type { ListUsersQuery } from "@/lib/api/types";

/**
 * Query keys for users
 */
export const usersKeys = {
  all: ["users"] as const,
  list: (query?: ListUsersQuery) => [...usersKeys.all, "list", query] as const,
};

/**
 * Hook to list users with optional search
 */
export function useUsers(query?: ListUsersQuery) {
  return useQuery({
    queryKey: usersKeys.list(query),
    queryFn: () => listUsers(query),
  });
}
