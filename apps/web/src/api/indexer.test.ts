import { afterEach, describe, expect, it, vi } from "vitest";

import { fetchRoomHistory } from "./indexer";

describe("fetchRoomHistory", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("walks the indexed room cursor until the retained history is complete", async () => {
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockImplementationOnce(async (input) => {
        expect(input).toBe("/api/room/messages?limit=100");

        return new Response(
          JSON.stringify({
            messages: [
              {
                messageId: "room-message-9",
                seq: 9,
                authorCanisterId: "rrkah-fqaaa-aaaaa-aaaaq-cai",
                createdAt: 1_710_000_000_000,
                body: "history start",
                mentions: [],
                contentType: "text/plain"
              },
              {
                messageId: "room-message-10",
                seq: 10,
                authorCanisterId: "rrkah-fqaaa-aaaaa-aaaaq-cai",
                createdAt: 1_710_000_000_100,
                body: "history continues",
                mentions: [],
                contentType: "text/plain"
              }
            ],
            nextAfterSeq: 10,
            latestSeq: 12
          }),
          {
            status: 200,
            headers: {
              "content-type": "application/json"
            }
          }
        );
      })
      .mockImplementationOnce(async (input) => {
        expect(input).toBe("/api/room/messages?afterSeq=10&limit=100");

        return new Response(
          JSON.stringify({
            messages: [
              {
                messageId: "room-message-11",
                seq: 11,
                authorCanisterId: "ryjl3-tyaaa-aaaaa-aaaba-cai",
                createdAt: 1_710_000_000_200,
                body: "history end",
                mentions: ["rrkah-fqaaa-aaaaa-aaaaq-cai"],
                contentType: "application/json"
              }
            ],
            nextAfterSeq: null,
            latestSeq: 12
          }),
          {
            status: 200,
            headers: {
              "content-type": "application/json"
            }
          }
        );
      });

    const page = await fetchRoomHistory();

    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(page).toEqual({
      messages: [
        expect.objectContaining({ messageId: "room-message-9", seq: 9 }),
        expect.objectContaining({ messageId: "room-message-10", seq: 10 }),
        expect.objectContaining({ messageId: "room-message-11", seq: 11 })
      ],
      nextAfterSeq: null,
      latestSeq: 12
    });
  });
});
