import { describe, expect, it, vi } from "vitest";

import {
  decodeAttentionPrice,
  encodeQueueMessageEthData,
  encodeQueueMessageData,
  readAttentionPrice,
  sendDirectPatronage,
  sendPaidMessage
} from "./patronage";

const address = "0x1111111111111111111111111111111111111111";

describe("patronage chain calls", () => {
  it("encodes dynamic paid messages and decodes contract quotes", () => {
    expect(encodeQueueMessageData(address, "hello", 1_500_000n)).toMatch(/^0xdc0a1b6a/);
    expect(decodeAttentionPrice(`0x${"1".padStart(64, "0")}${"2".padStart(64, "0")}${"1".padStart(64, "0")}`)).toEqual({
      usdcRaw: 1n,
      ethWei: 2n,
      usesDefault: true
    });
  });

  it("approves USDC before queuing a paid message", async () => {
    const request = vi.fn().mockResolvedValueOnce("0xapprove").mockResolvedValueOnce("0xmessage");
    const result = await sendPaidMessage({
      asset: "usdc",
      automatonAddress: address,
      inboxAddress: "0x2222222222222222222222222222222222222222",
      message: "A paid field note",
      price: { usdcRaw: 1_500_000n, ethWei: 10n, usesDefault: false },
      usdcAddress: "0x3333333333333333333333333333333333333333",
      wallet: { address, chainId: 8453, request } as never,
      expectedChainId: 8453
    });
    expect(result).toEqual(["0xapprove", "0xmessage"]);
    expect(request.mock.calls).toEqual([
      [{
        method: "eth_sendTransaction",
        params: [{
          from: address,
          to: "0x3333333333333333333333333333333333333333",
          data: `0x095ea7b3${"2222222222222222222222222222222222222222".padStart(64, "0")}${(1_500_000n).toString(16).padStart(64, "0")}`
        }]
      }],
      [{
        method: "eth_sendTransaction",
        params: [{
          from: address,
          to: "0x2222222222222222222222222222222222222222",
          value: "0xa",
          data: encodeQueueMessageData(address, "A paid field note", 1_500_000n)
        }]
      }]
    ]);
  });

  it("sends an ETH-only paid message with exact transaction parameters", async () => {
    const request = vi.fn().mockResolvedValue("0xmessage");
    await sendPaidMessage({
      asset: "eth",
      automatonAddress: address,
      inboxAddress: "0x2222222222222222222222222222222222222222",
      message: "An ETH field note",
      price: { usdcRaw: 1_500_000n, ethWei: 500_000_000_000_000n, usesDefault: true },
      usdcAddress: null,
      wallet: { address, chainId: 8453, request } as never,
      expectedChainId: 8453
    });
    expect(request).toHaveBeenCalledWith({
      method: "eth_sendTransaction",
      params: [{
        from: address,
        to: "0x2222222222222222222222222222222222222222",
        value: "0x1c6bf52634000",
        data: encodeQueueMessageEthData(address, "An ETH field note")
      }]
    });
  });

  it("reads the public quote over JSON-RPC without a wallet", async () => {
    const result = `0x${"1".padStart(64, "0")}${"2".padStart(64, "0")}${"0".padStart(64, "0")}`;
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result }), { status: 200 }));
    await expect(readAttentionPrice({
      automatonAddress: address,
      inboxAddress: "0x2222222222222222222222222222222222222222",
      rpcUrl: "https://rpc.example.test"
    })).resolves.toEqual({ usdcRaw: 1n, ethWei: 2n, usesDefault: false });
    expect(fetchMock).toHaveBeenCalledWith("https://rpc.example.test", expect.objectContaining({ method: "POST" }));
    fetchMock.mockRestore();
  });

  it("registers USDC patronage through the verified inbox event path", async () => {
    const request = vi.fn().mockResolvedValue("0xgift");
    const inboxAddress = "0x2222222222222222222222222222222222222222";
    await sendDirectPatronage({
      amountRaw: 2_000_000n,
      automatonAddress: address,
      inboxAddress,
      usdcAddress: "0x3333333333333333333333333333333333333333",
      wallet: { address, chainId: 8453, request } as never,
      expectedChainId: 8453
    });
    expect(request.mock.calls[0]?.[0].params[0].to).toBe("0x3333333333333333333333333333333333333333");
    expect(request.mock.calls[1]?.[0].params[0]).toMatchObject({
      to: inboxAddress,
      data: expect.stringMatching(/^0x4984ea4a/)
    });
  });
});
