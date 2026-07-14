export declare const ROOM_CONTENT_TYPES: readonly ["text/plain", "application/json"];
export declare const ROOM_MESSAGE_SCOPES: readonly ["all", "relevant"];
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
    settlement?: RoomMessageSettlement;
}
export interface RoomMessageSettlement {
    status: "not_claimed" | "unsettled" | "settled";
    txHash: string | null;
    payerCanisterId: string | null;
    payeeCanisterId: string | null;
    asset: "eth" | "usdc" | null;
    amountRaw: string | null;
    verifiedAt: number | null;
    provenance: string | null;
}
export interface PaymentGraphEdge {
    fromCanisterId: string;
    toCanisterId: string;
    asset: "eth" | "usdc";
    amountRaw: string;
    transactionCount: number;
    txHashes: string[];
}
export interface PaymentGraphResponse {
    from: number;
    to: number;
    edges: PaymentGraphEdge[];
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
//# sourceMappingURL=room.d.ts.map
