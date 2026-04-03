import { renderToStaticMarkup } from "react-dom/server";
import type { ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";

import { FundStep } from "./FundStep";

function createBaseProps(): ComponentProps<typeof FundStep> {
  return {
    asset: "usdc" as const,
    grossAmountInput: "100",
    preview: {
      creationCostDisplay: "8.00 USDC",
      creationCostUsd: 8,
      grossAmount: 100,
      grossDisplay: "100.00 USDC",
      grossUsd: 100,
      minimumMet: true,
      minimumUsd: 50,
      netForwardDisplay: "87.50 USDC",
      netForwardUsd: 87.5,
      platformFeeDisplay: "4.50 USDC",
      platformFeeUsd: 4.5
    },
    validationMessage: "Gross payment clears the $50.00 minimum.",
    summary: {
      braveConfigured: false,
      chain: "Base",
      providerModel: "Inference disabled until steward config",
      risk: "Balanced",
      strategies: "Base Aave USDC Reserve"
    },
    balances: {
      errorMessage: null,
      ethBalance: "6.9996 ETH",
      ethStatus: "Ready for gas",
      isLoading: false,
      usdcBalance: "400 USDC",
      usdcStatus: "Ready for payment"
    },
    faucet: {
      actionLabel: "Get test funds",
      disabledReason: null,
      errorMessage: null,
      isPending: false,
      statusMessage: "Faucet sends 1 ETH + 250 USDC.",
      txLinks: []
    },
    network: {
      actionLabel: "Add / switch playground network",
      disabled: true,
      errorMessage: null,
      isPending: false,
      statusMessage: "Wallet is already on Automaton Playground."
    },
    onAssetChange: vi.fn(),
    onClaimFaucet: vi.fn(),
    onConnectWallet: vi.fn(),
    onGrossAmountChange: vi.fn(),
    onNetworkAction: vi.fn(),
    onProviderChange: vi.fn(),
    playground: {
      chainId: 8453,
      chainName: "Automaton Playground",
      environmentLabel: "Automaton Playground",
      maintenance: false,
      note: "Canisters, balances, and session state are non-durable in this playground.",
      runtimeError: null,
      usesFallback: false
    },
    wallet: {
      address: "0x123",
      connectLabel: "Wallet connected",
      errorMessage: null,
      hasProvider: true,
      isConnecting: false,
      providerOptions: [
        {
          icon: null,
          id: "metamask",
          kind: "legacy",
          name: "MetaMask",
          provider: {
            request: vi.fn()
          },
          rdns: "io.metamask"
        }
      ],
      selectedProviderId: "metamask",
      statusMessage: "MetaMask is connected for playground funding and payment."
    }
  };
}

describe("FundStep", () => {
  it("hides onboarding cards once the wallet is connected, funded, and on the right network", () => {
    const markup = renderToStaticMarkup(<FundStep {...createBaseProps()} />);

    expect(markup).not.toContain("Choose provider");
    expect(markup).not.toContain("Add / switch network");
    expect(markup).not.toContain("Get test funds");
    expect(markup).not.toContain("Playground wallet state");
  });

  it("shows only the actionable onboarding cards for the current state", () => {
    const markup = renderToStaticMarkup(
      <FundStep
        {...createBaseProps()}
        balances={{
          errorMessage: null,
          ethBalance: "0.001 ETH",
          ethStatus: "Insufficient ETH for gas",
          isLoading: false,
          usdcBalance: "20 USDC",
          usdcStatus: "Insufficient USDC"
        }}
        network={{
          actionLabel: "Add / switch playground network",
          disabled: false,
          errorMessage: null,
          isPending: false,
          statusMessage: "Wallet is connected to chain 1. Switch to Automaton Playground before spawning."
        }}
      />
    );

    expect(markup).not.toContain("Choose provider");
    expect(markup).toContain("Add / switch network");
    expect(markup).not.toContain("Get test funds");
    expect(markup).not.toContain("Playground wallet state");
  });
});
