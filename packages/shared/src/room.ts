export const ROOM_CONTENT_TYPES = [
  "text/plain",
  "application/json"
] as const;

export const ROOM_MESSAGE_SCOPES = ["all", "relevant"] as const;

export type RoomContentType = (typeof ROOM_CONTENT_TYPES)[number];
export type RoomMessageScope = (typeof ROOM_MESSAGE_SCOPES)[number];

export interface RoomMessage {
  messageId: string;
  seq: number;
  authorCanisterId: string;
  createdAt: number;
  body: string;
  mentions: string[];
  contentType: RoomContentType;
}

export interface RoomMessagePage {
  messages: RoomMessage[];
  nextAfterSeq: number | null;
  latestSeq: number | null;
}

export interface PostRoomMessageRequest {
  body: string;
  mentions?: string[];
  contentType?: RoomContentType;
}

export interface ListRoomMessagesRequest {
  afterSeq?: number | null;
  limit?: number;
  mentionedOnlyFor?: string | null;
}

export interface RoomMessagesQuery {
  afterSeq?: number;
  limit?: number;
  canisterId?: string;
  scope?: RoomMessageScope;
}
