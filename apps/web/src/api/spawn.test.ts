import { beforeEach, describe, expect, it, vi } from "vitest";

import { refundSpawnSession, retrySpawnSession } from "./spawn";

const address = "0x1111111111111111111111111111111111111111";
const template = {
  signingPayload: "ic-automaton:factory-steward:v1\npayload",
  chainId: "8453",
  address,
  commandHash: `0x${"11".repeat(32)}`,
  nonce: "7",
  expiresAtNs: "9999999999999999999"
};

function response(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
}

describe("factory steward command signing", () => {
  beforeEach(() => vi.restoreAllMocks());

  it("signs only the factory-provided payload and forwards every proof field", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(response(template))
      .mockResolvedValueOnce(response({ session: { sessionId: "session-1" } }));
    const request = vi.fn().mockResolvedValue("0xsignature");
    await retrySpawnSession("session-1", address, request);
    const expectedPayloadHex = `0x${Array.from(new TextEncoder().encode(template.signingPayload), (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
    expect(request).toHaveBeenCalledWith({
      method: "personal_sign",
      params: [expectedPayloadHex, address]
    });
    const submitted = JSON.parse(String(fetchMock.mock.calls[1]?.[1]?.body));
    expect(submitted).toEqual({
      command: { retrySpawnSession: { sessionId: "session-1" } },
      proof: { chainId: "8453", address, commandHash: template.commandHash, nonce: "7", expiresAtNs: template.expiresAtNs, signature: "0xsignature" }
    });
  });

  it("rejects wallet mismatch before requesting a signature", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(response(template));
    const request = vi.fn();
    await expect(retrySpawnSession("session-1", `0x${"22".repeat(20)}`, request)).rejects.toThrow("not the session steward");
    expect(request).not.toHaveBeenCalled();
  });

  it("propagates user-cancelled signatures", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(response(template));
    await expect(retrySpawnSession("session-1", address, vi.fn().mockRejectedValue(new Error("User rejected")))).rejects.toThrow("User rejected");
  });

  it("rejects an expired prepared payload", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(response({ ...template, expiresAtNs: "1" }));
    await expect(retrySpawnSession("session-1", address, vi.fn())).rejects.toThrow("expired");
  });

  it("reconciles a lost response only when the factory nonce advanced", async () => {
    const detail = { session: { sessionId: "session-1", paymentStatus: "paid" } };
    vi.spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(response(template))
      .mockRejectedValueOnce(new Error("connection lost"))
      .mockResolvedValueOnce(response(detail))
      .mockResolvedValueOnce(response({ ...template, nonce: "8" }));
    await expect(retrySpawnSession("session-1", address, vi.fn().mockResolvedValue("0xsig"))).resolves.toEqual({ session: detail.session });
  });

  it("does not retry or claim success after a lost response with unchanged nonce", async () => {
    vi.spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(response(template))
      .mockRejectedValueOnce(new Error("connection lost"))
      .mockResolvedValueOnce(response({ session: { sessionId: "session-1" } }))
      .mockResolvedValueOnce(response(template));
    await expect(retrySpawnSession("session-1", address, vi.fn().mockResolvedValue("0xsig"))).rejects.toThrow("connection lost");
  });

  it("never fabricates refund receipt fields after a lost accepted response", async () => {
    vi.spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(response(template))
      .mockRejectedValueOnce(new Error("connection lost"))
      .mockResolvedValueOnce(response({ session: { sessionId: "session-1", paymentStatus: "refunded" } }))
      .mockResolvedValueOnce(response({ ...template, nonce: "8" }));
    await expect(refundSpawnSession("session-1", address, vi.fn().mockResolvedValue("0xsig")))
      .rejects.toThrow("authoritative refund receipt details");
  });
});
