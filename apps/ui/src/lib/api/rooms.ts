// Rooms: org conversations of people and agents (knowledge/runtime-resources/rooms.md).
// The console reads them for inspection and drives Playground rooms; apps build
// their own room UIs on the same API.

import { api } from "./client";
import type {
  LoopLimits,
  PostRoomMessageResponse,
  Room,
  RoomAuthor,
  RoomEvent,
  RoomEventListResponse,
  RoomFile,
  RoomFileContent,
  RoomFileHistoryResponse,
  RoomFileListResponse,
  RoomFileVersion,
  RoomListResponse,
  RoomMember,
  RoomMemberKind,
  RoomMemberListResponse,
  RoomMessage,
  RoomMessageListResponse,
  RoomWake,
  SessionScope,
  WakeMode,
} from "./schema-types";

export type {
  LoopLimits,
  PostRoomMessageResponse,
  Room,
  RoomAuthor,
  RoomEvent,
  RoomFile,
  RoomFileContent,
  RoomFileVersion,
  RoomMember,
  RoomMemberKind,
  RoomMessage,
  RoomWake,
  SessionScope,
  WakeMode,
};

/** A member to add when creating a room, as the API takes it. */
export interface RoomMemberInput {
  kind: RoomMemberKind;
  user_id?: string;
  agent_id?: string;
  handle?: string;
  display_name?: string;
  role?: "owner" | "member";
  wake?: WakeMode;
  session_scope?: SessionScope;
  instructions?: string;
}

export interface CreateRoomRequest {
  name: string;
  topic?: string;
  metadata?: Record<string, unknown>;
  members?: RoomMemberInput[];
}

const base = (roomId: string) => `/v1/rooms/${encodeURIComponent(roomId)}`;

export async function listRooms(
  options: { includeArchived?: boolean; mine?: boolean; limit?: number } = {},
): Promise<Room[]> {
  const params = new URLSearchParams({ limit: String(options.limit ?? 200) });
  if (options.includeArchived) params.set("include_archived", "true");
  if (options.mine) params.set("mine", "true");
  const response = await api.get<RoomListResponse>(`/v1/rooms?${params}`);
  return response.data.data;
}

export async function getRoom(roomId: string): Promise<Room> {
  return (await api.get<Room>(base(roomId))).data;
}

export async function createRoom(request: CreateRoomRequest): Promise<Room> {
  return (await api.post<Room>("/v1/rooms", request)).data;
}

export async function archiveRoom(roomId: string, archived: boolean): Promise<Room> {
  return (await api.post<Room>(`${base(roomId)}/archive`, { archived })).data;
}

export async function listRoomMembers(roomId: string, includeLeft = true): Promise<RoomMember[]> {
  const params = new URLSearchParams({ include_left: String(includeLeft) });
  const response = await api.get<RoomMemberListResponse>(`${base(roomId)}/members?${params}`);
  return response.data.data;
}

export async function addRoomMember(roomId: string, member: RoomMemberInput): Promise<RoomMember> {
  return (await api.post<RoomMember>(`${base(roomId)}/members`, { member })).data;
}

export async function updateRoomMember(
  roomId: string,
  memberId: string,
  change: { wake?: WakeMode; session_scope?: SessionScope },
): Promise<RoomMember> {
  const path = `${base(roomId)}/members/${encodeURIComponent(memberId)}`;
  return (await api.patch<RoomMember>(path, change)).data;
}

/** The newest page of top-level messages, oldest first. */
export async function listRoomMessages(roomId: string, limit = 200): Promise<RoomMessage[]> {
  const params = new URLSearchParams({ limit: String(limit) });
  const response = await api.get<RoomMessageListResponse>(`${base(roomId)}/messages?${params}`);
  return response.data.data;
}

export async function listRoomReplies(
  roomId: string,
  messageId: string,
  limit = 200,
): Promise<RoomMessage[]> {
  const params = new URLSearchParams({ limit: String(limit) });
  const path = `${base(roomId)}/messages/${encodeURIComponent(messageId)}/replies?${params}`;
  return (await api.get<RoomMessageListResponse>(path)).data.data;
}

export async function postRoomMessage(
  roomId: string,
  message: { text: string; reply_to?: string; as_member?: string },
): Promise<PostRoomMessageResponse> {
  return (await api.post<PostRoomMessageResponse>(`${base(roomId)}/messages`, message)).data;
}

/** The whole change feed of a room, in order, read page by page up to `max` events. */
export async function listRoomEvents(roomId: string, max = 2000): Promise<RoomEvent[]> {
  const events: RoomEvent[] = [];
  let after = 0;
  while (events.length < max) {
    const params = new URLSearchParams({ after: String(after), limit: "500" });
    const page = (await api.get<RoomEventListResponse>(`${base(roomId)}/events?${params}`)).data;
    events.push(...page.data);
    if (page.data.length < 500 || page.next_after <= after) break;
    after = page.next_after;
  }
  return events;
}

export async function listRoomFiles(roomId: string): Promise<RoomFile[]> {
  return (await api.get<RoomFileListResponse>(`${base(roomId)}/files`)).data.data;
}

export async function readRoomFile(
  roomId: string,
  path: string,
  version?: number,
): Promise<RoomFileContent> {
  const params = new URLSearchParams({ path });
  if (version !== undefined) params.set("version", String(version));
  return (await api.get<RoomFileContent>(`${base(roomId)}/files/content?${params}`)).data;
}

export async function listRoomFileHistory(
  roomId: string,
  path: string,
): Promise<RoomFileVersion[]> {
  const params = new URLSearchParams({ path });
  const response = await api.get<RoomFileHistoryResponse>(
    `${base(roomId)}/files/history?${params}`,
  );
  return response.data.data;
}
