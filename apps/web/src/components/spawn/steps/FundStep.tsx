import type { SpawnAsset } from "../../../../../../packages/shared/src/spawn.js";

import type { WalletProviderOption } from "../../../wallet/eip6963";
import type { FundingPreview } from "../spawn-state";

interface FundSummary {
  chain: string;
  risk: string;
  strategies: number;
  skills: number;
  providerModel: string;
  braveConfigured: boolean;
}

interface FundStepWalletStatus {
  address: string | null;
  connectLabel: string;
  errorMessage: string | null;
  hasProvider: boolean;
  isConnecting: boolean;
  providerOptions: WalletProviderOption[];
  selectedProviderId: string | null;
  statusMessage: string;
}

interface FundStepNetworkStatus {
  actionLabel: string;
  disabled: boolean;
  errorMessage: string | null;
  isPending: boolean;
  statusMessage: string;
}

interface FundStepFaucetTransactionLink {
  asset: "eth" | "usdc";
  hash: string;
  href: string | null;
}

interface FundStepFaucetStatus {
  actionLabel: string;
  disabledReason: string | null;
  errorMessage: string | null;
  isPending: boolean;
  statusMessage: string;
  txLinks: FundStepFaucetTransactionLink[];
}

interface FundStepBalanceStatus {
  errorMessage: string | null;
  ethBalance: string;
  ethStatus: string;
  isLoading: boolean;
  usdcBalance: string;
  usdcStatus: string;
}

interface FundStepPlaygroundStatus {
  chainId: number | null;
  chainName: string;
  environmentLabel: string;
  maintenance: boolean;
  note: string;
  runtimeError: string | null;
  usesFallback: boolean;
}

interface FundStepProps {
  asset: SpawnAsset;
  grossAmountInput: string;
  preview: FundingPreview;
  validationMessage: string;
  summary: FundSummary;
  balances: FundStepBalanceStatus;
  faucet: FundStepFaucetStatus;
  network: FundStepNetworkStatus;
  onAssetChange: (asset: SpawnAsset) => void;
  onClaimFaucet: () => void;
  onConnectWallet: () => void;
  onGrossAmountChange: (value: string) => void;
  onNetworkAction: () => void;
  onProviderChange: (providerId: string) => void;
  playground: FundStepPlaygroundStatus;
  wallet: FundStepWalletStatus;
}

function formatChainId(value: number | null) {
  if (value === null) {
    return "Chain pending";
  }

  return `${value} / 0x${value.toString(16)}`;
}

export function FundStep({
  asset,
  grossAmountInput,
  preview,
  validationMessage,
  summary,
  balances,
  faucet,
  network,
  onAssetChange,
  onClaimFaucet,
  onConnectWallet,
  onGrossAmountChange,
  onNetworkAction,
  onProviderChange,
  playground,
  wallet
}: FundStepProps) {
  return (
    <section className="spawn-step">
      <p className="section-label">Step 4</p>
      <h3 className="spawn-step-title">Fund</h3>
      <p className="spawn-step-copy">
        Connect a wallet, add the playground network, top up test funds if
        needed, then confirm the quoted USDC payment.
      </p>

      <div className="spawn-onboarding-shell">
        <article
          className={`spawn-onboarding-card is-playground${playground.maintenance ? " is-maintenance" : ""}`}
        >
          <div className="spawn-onboarding-header">
            <div>
              <p className="section-label">Playground / test environment</p>
              <h4 className="spawn-onboarding-title">
                {playground.environmentLabel}
              </h4>
            </div>
            <span className="spawn-onboarding-pill">
              {playground.maintenance ? "Maintenance active" : "Public test network"}
            </span>
          </div>

          <div className="spawn-onboarding-meta">
            <div className="spawn-onboarding-row">
              <span>Chain</span>
              <strong>{playground.chainName}</strong>
            </div>
            <div className="spawn-onboarding-row">
              <span>Chain ID</span>
              <strong>{formatChainId(playground.chainId)}</strong>
            </div>
          </div>

          <p className="spawn-inline-note">{playground.note}</p>
          {playground.usesFallback ? (
            <p className="spawn-inline-note">
              Runtime playground metadata is unavailable, so the wizard is using
              local fallback chain values.
            </p>
          ) : null}
          {playground.runtimeError !== null ? (
            <p className="spawn-session-error" role="alert">
              {playground.runtimeError}
            </p>
          ) : null}
        </article>

        <div className="spawn-onboarding-grid">
          <article className="spawn-onboarding-card">
            <div className="spawn-onboarding-header">
              <div>
                <p className="section-label">Wallet</p>
                <h4 className="spawn-onboarding-title">Choose provider</h4>
              </div>
            </div>

            {wallet.providerOptions.length > 1 ? (
              <label className="spawn-field">
                <span className="spawn-field-label">Detected wallets</span>
                <select
                  className="spawn-select"
                  onChange={(event) => {
                    onProviderChange(event.currentTarget.value);
                  }}
                  value={
                    wallet.selectedProviderId ??
                    wallet.providerOptions[0]?.id ??
                    ""
                  }
                >
                  {wallet.providerOptions.map((provider) => (
                    <option key={provider.id} value={provider.id}>
                      {provider.name}
                    </option>
                  ))}
                </select>
              </label>
            ) : null}

            <p className="spawn-inline-note">{wallet.statusMessage}</p>
            {wallet.address !== null ? (
              <p className="spawn-inline-note">
                Connected wallet: {wallet.address}
              </p>
            ) : null}

            <button
              className="spawn-nav-button is-primary"
              disabled={
                !wallet.hasProvider || wallet.isConnecting || wallet.address !== null
              }
              onClick={onConnectWallet}
              type="button"
            >
              {wallet.isConnecting ? "Connecting wallet..." : wallet.connectLabel}
            </button>

            {wallet.errorMessage !== null ? (
              <p className="spawn-session-error" role="alert">
                {wallet.errorMessage}
              </p>
            ) : null}
          </article>

          <article className="spawn-onboarding-card">
            <div className="spawn-onboarding-header">
              <div>
                <p className="section-label">Network</p>
                <h4 className="spawn-onboarding-title">Add / switch network</h4>
              </div>
            </div>

            <p className="spawn-inline-note">{network.statusMessage}</p>

            <button
              className="spawn-nav-button is-primary"
              disabled={network.disabled || network.isPending}
              onClick={onNetworkAction}
              type="button"
            >
              {network.isPending ? "Updating network..." : network.actionLabel}
            </button>

            {network.errorMessage !== null ? (
              <p className="spawn-session-error" role="alert">
                {network.errorMessage}
              </p>
            ) : null}
          </article>

          <article className="spawn-onboarding-card">
            <div className="spawn-onboarding-header">
              <div>
                <p className="section-label">Faucet</p>
                <h4 className="spawn-onboarding-title">Get test funds</h4>
              </div>
            </div>

            <p className="spawn-inline-note">{faucet.statusMessage}</p>

            <button
              className="spawn-nav-button is-primary"
              disabled={faucet.disabledReason !== null || faucet.isPending}
              onClick={onClaimFaucet}
              type="button"
            >
              {faucet.isPending ? "Funding wallet..." : faucet.actionLabel}
            </button>

            {faucet.disabledReason !== null ? (
              <p className="spawn-inline-note">{faucet.disabledReason}</p>
            ) : null}

            {faucet.errorMessage !== null ? (
              <p className="spawn-session-error" role="alert">
                {faucet.errorMessage}
              </p>
            ) : null}

            {faucet.txLinks.length > 0 ? (
              <ul className="spawn-link-list">
                {faucet.txLinks.map((transaction) => (
                  <li key={`${transaction.asset}:${transaction.hash}`}>
                    <span>{transaction.asset.toUpperCase()}</span>
                    {transaction.href === null ? (
                      <strong>{transaction.hash}</strong>
                    ) : (
                      <a href={transaction.href} rel="noreferrer" target="_blank">
                        {transaction.hash}
                      </a>
                    )}
                  </li>
                ))}
              </ul>
            ) : null}
          </article>

          <article className="spawn-onboarding-card">
            <div className="spawn-onboarding-header">
              <div>
                <p className="section-label">Balances</p>
                <h4 className="spawn-onboarding-title">Playground wallet state</h4>
              </div>
            </div>

            {balances.isLoading ? (
              <p className="spawn-inline-note">Checking playground balances.</p>
            ) : (
              <div className="spawn-onboarding-meta">
                <div className="spawn-onboarding-row">
                  <span>ETH</span>
                  <strong>{balances.ethBalance}</strong>
                </div>
                <div className="spawn-onboarding-row">
                  <span>Status</span>
                  <strong>{balances.ethStatus}</strong>
                </div>
                <div className="spawn-onboarding-row">
                  <span>USDC</span>
                  <strong>{balances.usdcBalance}</strong>
                </div>
                <div className="spawn-onboarding-row">
                  <span>Status</span>
                  <strong>{balances.usdcStatus}</strong>
                </div>
              </div>
            )}

            {balances.errorMessage !== null ? (
              <p className="spawn-session-error" role="alert">
                {balances.errorMessage}
              </p>
            ) : null}
          </article>
        </div>
      </div>

      <div className="fund-shell">
        <label className="spawn-field fund-field">
          <span className="spawn-field-label">Gross payment</span>
          <div className="fund-input-wrap">
            <input
              className="spawn-input fund-input"
              inputMode="decimal"
              onChange={(event) => {
                onGrossAmountChange(event.currentTarget.value);
              }}
              placeholder="0.00"
              type="number"
              value={grossAmountInput}
            />
            <div
              aria-label="Funding asset"
              className="fund-asset-toggle"
              role="group"
            >
              <button
                className={`fund-currency${asset === "usdc" ? " is-active" : ""}`}
                onClick={() => {
                  onAssetChange("usdc");
                }}
                type="button"
              >
                USDC
              </button>
            </div>
          </div>
        </label>

        <p className="fund-usd-copy">
          Approximate gross payment value: ${preview.grossUsd.toFixed(2)} USD
        </p>
        <p
          className={`fund-validation${preview.minimumMet ? "" : " has-error"}`}
          role="status"
        >
          {validationMessage}
        </p>

        <div className="fund-breakdown">
          <div className="fund-breakdown-row">
            <span>Gross payment</span>
            <strong>{preview.grossDisplay}</strong>
          </div>
          <div className="fund-breakdown-row">
            <span>Platform fee</span>
            <strong>{preview.platformFeeDisplay}</strong>
          </div>
          <div className="fund-breakdown-row">
            <span>Creation cost</span>
            <strong>{preview.creationCostDisplay}</strong>
          </div>
          <div className="fund-breakdown-row is-total">
            <span>Net forwarded to automaton</span>
            <strong>{preview.netForwardDisplay}</strong>
          </div>
        </div>

        <div className="fund-summary">
          <p className="section-label">Spawn summary</p>
          <div className="fund-summary-grid">
            <div className="fund-summary-row">
              <span>Chain</span>
              <strong>{summary.chain}</strong>
            </div>
            <div className="fund-summary-row">
              <span>Risk</span>
              <strong>{summary.risk}</strong>
            </div>
            <div className="fund-summary-row">
              <span>Strategies</span>
              <strong>{summary.strategies} selected</strong>
            </div>
            <div className="fund-summary-row">
              <span>Skills</span>
              <strong>{summary.skills} selected</strong>
            </div>
            <div className="fund-summary-row">
              <span>Model</span>
              <strong>{summary.providerModel}</strong>
            </div>
            <div className="fund-summary-row">
              <span>Brave Search</span>
              <strong>
                {summary.braveConfigured ? "Configured" : "Disabled for now"}
              </strong>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}
