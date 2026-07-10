import { beforeEach, describe, expect, it, vi } from "vitest";

import { executeStewardCommand } from "./steward-command-executor";

const proof = {
  canister_id: "aaaaa-aa",
  chain_id: 8453,
  address: "0xAbCdEfAbCdEfAbCdEfAbCdEfAbCdEfAbCdEfAbCd",
  command_hash: "0xcommand",
  nonce: 17,
  expires_at_ns: "1800000000000"
};

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" }
  });
}

function createContext(request = vi.fn().mockResolvedValue("0xsignature")) {
  return {
    automaton: null,
    canisterUrl: "https://automaton.example/",
    connectedAddress: proof.address,
    connectedChainId: 8453,
    request,
    refreshStewardStatus: vi.fn().mockResolvedValue({ next_nonce: 17 }),
    sleep: vi.fn().mockResolvedValue(undefined)
  };
}

describe("steward command executor", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it("prepares, signs the exact UTF-8 payload, and executes steward-send", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
      jsonResponse({ sender: proof.address, message: "héllo", proof_template: proof, signing_payload: "héllo|17" })
    ).mockResolvedValueOnce(jsonResponse({ result: "queued" }));
    const request = vi.fn().mockResolvedValue("0xsig");

    const result = await executeStewardCommand('steward-send -m "héllo"', createContext(request));

    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      new URL("https://automaton.example/api/steward/direct-message/prepare"),
      expect.objectContaining({ body: JSON.stringify({ sender: proof.address, message: "héllo" }) })
    );
    expect(request).toHaveBeenCalledWith({
      method: "personal_sign",
      params: ["0x68c3a96c6c6f7c3137", proof.address]
    });
    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      new URL("https://automaton.example/api/steward/direct-message/execute"),
      expect.objectContaining({
        body: JSON.stringify({
          sender: proof.address,
          message: "héllo",
          proof: { ...proof, signature: "0xsig" }
        })
      })
    );
    expect(result.entries.at(-1)?.text).toBe("queued");
  });

  it("maps model and reasoning commands using prepared normalized values", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(jsonResponse({ model: "normalized/model", proof_template: proof, signing_payload: "model" }))
      .mockResolvedValueOnce(jsonResponse({ result: "model applied" }))
      .mockResolvedValueOnce(jsonResponse({ variant: "medium", proof_template: { ...proof, nonce: 18 }, signing_payload: "reasoning" }))
      .mockResolvedValueOnce(jsonResponse({ result: "reasoning applied" }));
    const context = createContext();

    await executeStewardCommand("steward-model requested/model", context);
    await executeStewardCommand("steward-reasoning medium", context);

    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      new URL("https://automaton.example/api/steward/model/execute"),
      expect.objectContaining({ body: expect.stringContaining('"model":"normalized/model"') })
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      4,
      new URL("https://automaton.example/api/steward/reasoning/execute"),
      expect.objectContaining({ body: expect.stringContaining('"variant":"medium"') })
    );
  });

  it("rejects malformed preparation, wrong signer context, and wallet rejection", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(jsonResponse({ proof_template: proof }));
    const malformed = await executeStewardCommand("steward-model x", createContext());
    expect(malformed.entries.at(-1)?.kind).toBe("error");

    vi.restoreAllMocks();
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
      jsonResponse({ proof_template: { ...proof, chain_id: 1 }, signing_payload: "payload" })
    );
    const wrongChain = await executeStewardCommand("steward-model x", createContext());
    expect(wrongChain.entries.at(-1)?.text).toContain("chain");

    vi.restoreAllMocks();
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(
      jsonResponse({ proof_template: proof, signing_payload: "payload" })
    );
    const rejected = await executeStewardCommand(
      "steward-model x",
      createContext(vi.fn().mockRejectedValue(new Error("User rejected the request")))
    );
    expect(rejected.entries.at(-1)?.text).toContain("rejected");
  });

  it("does not retry execute 4xx responses", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(jsonResponse({ proof_template: proof, signing_payload: "payload" }))
      .mockResolvedValueOnce(jsonResponse({ error: "bad request" }, 400));

    const result = await executeStewardCommand("steward-model x", createContext());

    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(result.entries.at(-1)?.text).toContain("400");
  });

  it("retries a transient execute failure once", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(jsonResponse({ proof_template: proof, signing_payload: "payload" }))
      .mockResolvedValueOnce(jsonResponse({ error: "temporary" }, 503))
      .mockResolvedValueOnce(jsonResponse({ result: "applied after retry" }));

    const result = await executeStewardCommand("steward-model x", createContext());

    expect(fetchMock).toHaveBeenCalledTimes(3);
    expect(result.entries.at(-1)?.text).toBe("applied after retry");
  });

  it("reconciles a lost response only after nonce advancement", async () => {
    vi.spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(jsonResponse({ proof_template: proof, signing_payload: "payload" }))
      .mockRejectedValueOnce(new TypeError("network boundary"))
      .mockRejectedValueOnce(new TypeError("network boundary"));
    const context = createContext();
    context.refreshStewardStatus.mockResolvedValue({ next_nonce: 18 });

    const applied = await executeStewardCommand("steward-model x", context);
    expect(applied.entries.at(-1)?.text).toContain("nonce advanced");

    vi.restoreAllMocks();
    vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(jsonResponse({ proof_template: proof, signing_payload: "payload" }))
      .mockRejectedValueOnce(new TypeError("network boundary"))
      .mockRejectedValueOnce(new TypeError("network boundary"));
    context.refreshStewardStatus.mockResolvedValue({ next_nonce: 17 });
    const failed = await executeStewardCommand("steward-model x", context);
    expect(failed.entries.at(-1)?.kind).toBe("error");
  });
});
