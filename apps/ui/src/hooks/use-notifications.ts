import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { listNotifications, markNotificationViewed } from "@/lib/api/notifications";
import { queryKeys } from "@/lib/query-keys";

export function useNotifications(enabled: boolean = true) {
  return useQuery({
    queryKey: queryKeys.notifications.list(),
    queryFn: () => listNotifications(),
    enabled,
    staleTime: 60 * 1000,
  });
}

export function useMarkNotificationViewed() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: (notificationId: string) => markNotificationViewed(notificationId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.notifications.list() });
    },
  });
}
