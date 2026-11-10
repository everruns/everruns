"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  createRoom,
  getRoom,
  listRoomEvents,
  listRoomMembers,
  listRoomMessages,
  listRooms,
  postRoomMessage,
  type CreateRoomRequest,
} from "@/lib/api/rooms";
import { useOrg } from "@/providers/org-provider";

export const roomKeys = {
  all: ["rooms"] as const,
  list: (org: string | undefined, archived: boolean) => ["rooms", "list", org, archived] as const,
  room: (roomId: string) => ["rooms", roomId] as const,
};

export function useRooms(options: { includeArchived?: boolean } = {}) {
  const { currentOrg } = useOrg();
  const org = currentOrg?.public_id;
  const includeArchived = options.includeArchived ?? false;
  return useQuery({
    queryKey: roomKeys.list(org, includeArchived),
    queryFn: () => listRooms({ includeArchived }),
    enabled: !!org,
  });
}

/**
 * One room with everything its views read: members, top-level messages and
 * the change feed. `poll` refetches while agents may still be answering.
 */
export function useRoom(roomId: string, options: { poll?: number | false } = {}) {
  const refetchInterval = options.poll ?? false;
  const room = useQuery({
    queryKey: [...roomKeys.room(roomId), "room"],
    queryFn: () => getRoom(roomId),
  });
  const members = useQuery({
    queryKey: [...roomKeys.room(roomId), "members"],
    queryFn: () => listRoomMembers(roomId),
    enabled: room.isSuccess,
    refetchInterval,
  });
  const messages = useQuery({
    queryKey: [...roomKeys.room(roomId), "messages"],
    queryFn: () => listRoomMessages(roomId),
    enabled: room.isSuccess,
    refetchInterval,
  });
  const events = useQuery({
    queryKey: [...roomKeys.room(roomId), "events"],
    queryFn: () => listRoomEvents(roomId),
    enabled: room.isSuccess,
    refetchInterval,
  });
  return { room, members, messages, events };
}

export function useCreateRoom() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: CreateRoomRequest) => createRoom(request),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: roomKeys.all }),
  });
}

export function usePostRoomMessage(roomId: string) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (message: { text: string; reply_to?: string; as_member?: string }) =>
      postRoomMessage(roomId, message),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: roomKeys.room(roomId) }),
  });
}
