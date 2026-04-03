import { afterEach, describe, expect, it, vi } from "vitest";

import { fetchRepositoryStrategies, fetchRoomHistory } from "./indexer";

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

describe("fetchRepositoryStrategies", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("reads repository-backed strategy listings from the indexer", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(
        JSON.stringify({
          items: [
            {
              strategyId: "base-aave-usdc-reserve-01",
              name: "Base Aave USDC Reserve",
              description: "Park surplus Base USDC on Aave V3.",
              canonicalChain: "base",
              canonicalChainId: 8453,
              compatibleSpawnChains: ["base"],
              protocol: "aave-v3",
              primitive: "lend_supply",
              recipeJson: "{}",
              status: "active",
              source: {
                sourcePath: "docs/strategies/base-aave-usdc-reserve-01/recipe.json",
                sourceCommit: "03961659ec3b86f8586ac07e5f295084bb6f6ffa"
              },
              createdAt: 1,
              updatedAt: 1,
              deprecatedAt: null,
              revokedAt: null
            }
          ],
          updatedAt: 1
        }),
        {
          status: 200,
          headers: {
            "content-type": "application/json"
          }
        }
      )
    );

    const response = await fetchRepositoryStrategies();

    expect(fetchMock).toHaveBeenCalledWith("/api/repository/strategies", expect.any(Object));
    expect(response.items[0]).toMatchObject({
      strategyId: "base-aave-usdc-reserve-01",
      status: "active"
    });
  });
});
