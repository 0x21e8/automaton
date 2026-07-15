// @vitest-environment happy-dom

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AutomatonDetail, PlaygroundMetadata } from "@ic-automaton/shared";
import type { WalletSession } from "../../wallet/useWalletSession";
import { PatronagePanel } from "./PatronagePanel";

const AUTOMATON = "0x1111111111111111111111111111111111111111";
const INBOX = "0x2222222222222222222222222222222222222222";
const USDC = "0x3333333333333333333333333333333333333333";
const SENDER = "0x4444444444444444444444444444444444444444";
const QUOTE = `0x${(1_500_000n).toString(16).padStart(64, "0")}${(500_000_000_000_000n).toString(16).padStart(64, "0")}${"0".padStart(64, "0")}`;

const automaton = {
  canisterId: "txyno-ch777-77776-aaaaq-cai",
  chain: "base",
  chainId: 8453,
  ethAddress: AUTOMATON,
  inboxContractAddress: INBOX,
  usdcContractAddress: USDC
} as AutomatonDetail;

const playgroundMetadata = {
  chain: {
    id: 8453,
    name: "Base",
    publicRpcUrl: "https://rpc.example.test",
    nativeCurrency: { name: "Ether", symbol: "ETH", decimals: 18 },
    explorerUrl: null
  }
} as PlaygroundMetadata;

function wallet(request: WalletSession["request"]): WalletSession {
  return {
    address: SENDER,
    chainId: 8453,
    hasProvider: true,
    request
  } as WalletSession;
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function mockQuote() {
  vi.spyOn(globalThis, "fetch").mockResolvedValue(
    new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result: QUOTE }), { status: 200 })
  );
}

describe("PatronagePanel interactions", () => {
  it("shows the public quote to a spectator without a wallet", async () => {
    mockQuote();
    render(<PatronagePanel automaton={automaton} playgroundMetadata={playgroundMetadata} wallet={null} />);

    expect(await screen.findByText(/Public quote: 1.5 USDC \+ 0.0005 ETH/)).toBeTruthy();
    expect(screen.getByText(/The public quote remains readable without a wallet/)).toBeTruthy();
  });

  it("loads a public quote and clicks the combined USDC + ETH paid-message flow", async () => {
    mockQuote();
    const request = vi.fn().mockResolvedValueOnce("0xapprove").mockResolvedValueOnce("0xmessage");
    render(<PatronagePanel automaton={automaton} playgroundMetadata={playgroundMetadata} wallet={wallet(request)} />);

    expect(await screen.findByText(/Public quote: 1.5 USDC \+ 0.0005 ETH/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Paid message"), { target: { value: "A paid field note" } });
    fireEvent.click(screen.getByRole("button", { name: /SEND · 1.5 USDC \+ 0.0005 ETH/ }));

    await waitFor(() => expect(request).toHaveBeenCalledTimes(2));
    expect(request.mock.calls[1]?.[0]).toMatchObject({
      method: "eth_sendTransaction",
      params: [{ from: SENDER, to: INBOX, value: "0x1c6bf52634000" }]
    });
    expect(await screen.findByText("Paid message submitted: 0xmessage")).toBeTruthy();
  });

  it("clicks the ETH-only paid-message flow", async () => {
    mockQuote();
    const request = vi.fn().mockResolvedValue("0xethmessage");
    render(<PatronagePanel automaton={automaton} playgroundMetadata={playgroundMetadata} wallet={wallet(request)} />);

    await screen.findByText(/Public quote:/);
    fireEvent.change(screen.getByLabelText("Payment asset"), { target: { value: "eth" } });
    fireEvent.change(screen.getByLabelText("Paid message"), { target: { value: "An ETH field note" } });
    fireEvent.click(screen.getByRole("button", { name: "SEND · 0.0005 ETH" }));

    await waitFor(() => expect(request).toHaveBeenCalledTimes(1));
    expect(request).toHaveBeenCalledWith(expect.objectContaining({
      method: "eth_sendTransaction",
      params: [expect.objectContaining({ from: SENDER, to: INBOX, value: "0x1c6bf52634000" })]
    }));
  });

  it("clicks direct USDC patronage to the being", async () => {
    mockQuote();
    const request = vi.fn().mockResolvedValue("0xgift");
    render(<PatronagePanel automaton={automaton} playgroundMetadata={playgroundMetadata} wallet={wallet(request)} />);

    await screen.findByText(/Public quote:/);
    fireEvent.change(screen.getByLabelText("Patronage amount in USDC"), { target: { value: "2.25" } });
    fireEvent.click(screen.getByRole("button", { name: "GIFT USDC" }));

    await waitFor(() => expect(request).toHaveBeenCalledTimes(2));
    expect(request).toHaveBeenNthCalledWith(1, expect.objectContaining({
      method: "eth_sendTransaction",
      params: [expect.objectContaining({ from: SENDER, to: USDC })]
    }));
    expect(request).toHaveBeenNthCalledWith(2, expect.objectContaining({
      method: "eth_sendTransaction",
      params: [expect.objectContaining({ from: SENDER, to: INBOX, data: expect.stringMatching(/^0x4984ea4a/) })]
    }));
    expect(await screen.findByText("Patronage submitted: 0xgift")).toBeTruthy();
  });
});
