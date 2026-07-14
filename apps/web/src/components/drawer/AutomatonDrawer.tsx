import { useEffect, useState } from "react";

import type { AutomatonDetail, PlaygroundMetadata } from "@ic-automaton/shared";
import { CommandLinePanel } from "./CommandLinePanel";
import { MonologuePanel } from "./MonologuePanel";
import type { WalletSession } from "../../wallet/useWalletSession";
import { MetabolismPanel } from "./MetabolismPanel";
import { PatronagePanel } from "./PatronagePanel";

function formatUsd(value: string | null): string {
  if (value === null) {
    return "n/a";
  }

  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency: "USD",
    maximumFractionDigits: 0
  }).format(Number(value));
}

function formatEth(wei: string | null): string {
  if (wei === null) {
    return "n/a";
  }

  return `${(Number(wei) / 1e18).toFixed(3)} ETH`;
}

function formatUsdc(raw: string | null): string {
  if (raw === null) {
    return "n/a";
  }

  let value: bigint;

  try {
    value = BigInt(raw);
  } catch {
    return "n/a";
  }

  const decimals = 6n;
  const divisor = 10n ** decimals;
  const whole = value / divisor;
  const fraction = (value % divisor).toString().padStart(Number(decimals), "0").slice(0, 3);

  return fraction === "000" ? `${whole.toString()} USDC` : `${whole.toString()}.${fraction} USDC`;
}

function formatCycles(value: number): string {
  if (value >= 1_000_000_000_000) {
    return `${(value / 1_000_000_000_000).toFixed(2)}T cycles`;
  }

  return `${value.toLocaleString("en-US")} cycles`;
}

function formatAddress(value: string): string {
  return `${value.slice(0, 6)}...${value.slice(-4)}`;
}

function formatLifetime(createdAt: number, now = Date.now()): string {
  const elapsedMs = Math.max(0, now - createdAt);
  const totalMinutes = Math.floor(elapsedMs / 60_000);
  const days = Math.floor(totalMinutes / (24 * 60));
  const hours = Math.floor((totalMinutes % (24 * 60)) / 60);
  const minutes = totalMinutes % 60;

  if (days > 0) {
    return `${days}d ${hours}h`;
  }

  if (hours > 0) {
    return `${hours}h ${minutes}m`;
  }

  return `${minutes}m`;
}

interface AutomatonDrawerProps {
  automaton: AutomatonDetail | null;
  errorMessage: string | null;
  isLoading: boolean;
  isOpen: boolean;
  onClose: () => void;
  selectedCanisterId: string | null;
  viewerAddress: string | null;
  walletSession: WalletSession | null;
  playgroundMetadata?: PlaygroundMetadata | null;
}

function isNotFoundError(errorMessage: string | null): boolean {
  return errorMessage?.toLowerCase().includes("not found") ?? false;
}

export function AutomatonDrawer({
  automaton,
  errorMessage,
  isLoading,
  isOpen,
  onClose,
  selectedCanisterId,
  viewerAddress,
  walletSession,
  playgroundMetadata = null
}: AutomatonDrawerProps) {
  const [copyLabel, setCopyLabel] = useState("COPY");
  const [activeSection, setActiveSection] = useState<"overview" | "activity" | "terminal" | "strategies">("overview");

  useEffect(() => {
    setCopyLabel("COPY");
    setActiveSection("overview");
  }, [automaton]);

  const canExecute =
    automaton !== null &&
    viewerAddress !== null &&
    automaton.steward.address.toLowerCase() === viewerAddress.toLowerCase();
  const detailMissing = isNotFoundError(errorMessage);

  async function copyAddress() {
    if (
      automaton === null ||
      automaton.ethAddress === null ||
      typeof navigator === "undefined" ||
      navigator.clipboard === undefined ||
      typeof window === "undefined"
    ) {
      return;
    }

    await navigator.clipboard.writeText(automaton.ethAddress);
    setCopyLabel("OK");

    window.setTimeout(() => {
      setCopyLabel("COPY");
    }, 1200);
  }

  const title = isLoading
    ? "Loading automaton detail"
    : errorMessage !== null
      ? detailMissing
        ? "Indexed automaton not found"
        : "Detail load failed"
      : automaton?.name ?? "Select an automaton";
  const tier = automaton?.tier ?? "normal";
  const detailFallbackCopy = isLoading
    ? "Loading the indexed canister snapshot."
    : errorMessage !== null
      ? detailMissing
        ? selectedCanisterId === null
          ? "The selected automaton is no longer indexed."
          : `No indexed detail is available for ${selectedCanisterId}.`
        : `Detail request failed: ${errorMessage}`
      : "Select an indexed automaton to inspect its canister.";
  const versionFallbackCopy = isLoading
    ? "Loading indexed version metadata."
    : errorMessage !== null
      ? detailMissing
        ? "Version metadata is unavailable because this automaton is not indexed."
        : "Version metadata is unavailable until the detail request succeeds."
      : "Commit metadata appears after selection.";

  return (
    <aside
      aria-hidden={!isOpen}
      className={`automaton-drawer${isOpen ? " is-open" : ""}`}
    >
      <div className="drawer-inner">
        <button className="close-btn" onClick={onClose} type="button">
          CLOSE ×
        </button>

        <div className="drawer-top">
          <h2>{title}</h2>
          <span className={`tier-pill tier-${tier}`}>
            {automaton?.tier ?? "standby"}
          </span>
          <span className="chain-badge">
            {automaton?.chain.toUpperCase() ?? "BASE"}
          </span>
        </div>

        <nav className="drawer-tabs" aria-label="Automaton sections">
          {(["overview", "activity", "terminal", "strategies"] as const).map((section) => (
            <button
              aria-selected={activeSection === section}
              className={`drawer-tab${activeSection === section ? " is-active" : ""}`}
              key={section}
              onClick={() => setActiveSection(section)}
              role="tab"
              type="button"
            >
              {section[0].toUpperCase() + section.slice(1)}
            </button>
          ))}
        </nav>

        {activeSection === "overview" ? <div className="drawer-grid">
          {automaton !== null ? <MetabolismPanel automaton={automaton} /> : null}
          <div>
            <div className="detail-field">
              <div className="lbl">ETH Address</div>
              <div className="addr-row">
                <span className="addr-text">
                  {isLoading
                    ? "Loading address"
                    : automaton?.ethAddress ?? "Not available yet"}
                </span>
                <button
                  className={`icon-btn${copyLabel === "OK" ? " copied" : ""}`}
                  disabled={automaton?.ethAddress === null || automaton === null}
                  onClick={() => {
                    void copyAddress();
                  }}
                  type="button"
                >
                  {copyLabel}
                </button>
                <a
                  className="icon-btn"
                  href={automaton?.explorerUrl ?? "#"}
                  rel="noreferrer"
                  target="_blank"
                >
                  SCAN
                </a>
              </div>
            </div>

            <div className="detail-field">
              <div className="lbl">Steward</div>
              <div className="addr-row">
                <span className="addr-text">
                  {automaton === null
                    ? isLoading
                      ? "Loading steward identity"
                      : errorMessage ?? "Select a live automaton"
                    : automaton.steward.ensName !== null
                      ? `${automaton.steward.ensName} ${formatAddress(automaton.steward.address)}`
                      : formatAddress(automaton.steward.address)}
                </span>
              </div>
            </div>

            <div className="detail-field">
              <div className="lbl">Canister</div>
              {automaton === null ? (
                <p className="empty-copy">{detailFallbackCopy}</p>
              ) : (
                <a
                  className="detail-link"
                  href={automaton.canisterUrl}
                  rel="noreferrer"
                  target="_blank"
                >
                  {automaton.canisterId}
                </a>
              )}
            </div>
          </div>

          <div>
            <div className="detail-field">
              <div className="lbl">ETH Balance</div>
              <div className="val">
                {automaton === null
                  ? isLoading
                    ? "loading"
                    : "n/a"
                  : formatEth(automaton.financials.ethBalanceWei)}
              </div>
            </div>

            <div className="detail-field">
              <div className="lbl">Cycles Balance</div>
              <div className="val">
                {automaton === null
                  ? isLoading
                    ? "loading"
                    : "n/a"
                  : formatCycles(automaton.financials.cyclesBalance)}
              </div>
            </div>

            <div className="detail-field">
              <div className="lbl">USDC Balance</div>
              <div className="val">
                {automaton === null
                  ? isLoading
                    ? "loading"
                    : "n/a"
                  : formatUsdc(automaton.financials.usdcBalanceRaw)}
              </div>
            </div>

            <div className="detail-field">
              <div className="lbl">Net Worth</div>
              <div className="val">
                {automaton === null
                  ? isLoading
                    ? "loading"
                    : "n/a"
                  : formatUsd(automaton.financials.netWorthUsd)}
              </div>
            </div>
          </div>

          <div>
            <div className="detail-field">
              <div className="lbl">Heartbeat</div>
              <div className="val">
                {automaton === null
                  ? isLoading
                    ? "loading"
                    : "n/a"
                  : `${automaton.runtime.heartbeatIntervalSeconds ?? "n/a"}s`}
              </div>
            </div>

            <div className="detail-field">
              <div className="lbl">Lifetime</div>
              <div className="val">
                {automaton === null
                  ? isLoading
                    ? "loading"
                    : "n/a"
                  : formatLifetime(automaton.createdAt)}
              </div>
            </div>

            <div className="detail-field">
              <div className="lbl">Model</div>
              <div className="val">
                {automaton === null
                  ? isLoading
                    ? "loading"
                    : "n/a"
                  : automaton.model ?? "Not configured"}
              </div>
            </div>

            <div className="detail-field">
              <div className="lbl">Version</div>
              {automaton === null ? (
                <p className="empty-copy">{versionFallbackCopy}</p>
              ) : (
                <a
                  className="detail-link"
                  href={`https://github.com/0x21e8/ic-automaton/commit/${automaton.version.commitHash}`}
                  rel="noreferrer"
                  target="_blank"
                >
                  {automaton.version.shortCommitHash}
                </a>
              )}
            </div>
          </div>
          {automaton !== null &&
          ((automaton.constitution !== null &&
            automaton.constitution !== undefined) ||
            automaton.constitutionVerification.status !== "unavailable") ? (
            <section className="detail-field">
              <div className="lbl">Founding document</div>
              {automaton.constitutionVerification.status === "mismatch" ? (
                <p role="alert">
                  Integrity warning: the child document does not match the factory
                  registry hash, so its contents are hidden.
                </p>
              ) : (
                <p>{automaton.constitution ?? "Founding document unavailable."}</p>
              )}
              {automaton.constitutionVerification.status === "verified" ? (
                <div className="addr-text">
                  Verified SHA-256 {automaton.constitutionHash}
                </div>
              ) : null}
              {automaton.constitutionVerification.status === "legacy_unverified" ? (
                <div className="addr-text" role="status">
                  Legacy document — no registry hash is available for verification.
                </div>
              ) : null}
            </section>
          ) : null}
          {automaton !== null ? <PatronagePanel automaton={automaton} playgroundMetadata={playgroundMetadata} wallet={walletSession} /> : null}
        </div> : null}

        <div className="drawer-bottom">
          {activeSection === "activity" ? (
            <MonologuePanel
              entries={automaton?.monologue ?? []}
              journalEntries={automaton?.journal ?? []}
              errorMessage={errorMessage}
              isLoading={isLoading}
              selectedCanisterId={selectedCanisterId}
            />
          ) : null}
          {activeSection === "terminal" ? (
            <CommandLinePanel
              automaton={automaton}
              canExecute={canExecute}
              enabled
              errorMessage={errorMessage}
              isLoading={isLoading}
              selectedCanisterId={selectedCanisterId}
              viewerAddress={viewerAddress}
              walletSession={walletSession}
            />
          ) : null}
          {activeSection === "strategies" ? (
            <section className="drawer-section" aria-labelledby="strategies-heading">
              <div className="panel-heading">
                <h3 id="strategies-heading">Strategies</h3>
                <span className="panel-note">Indexed configuration</span>
              </div>
              {automaton?.strategies.length ? (
                <ul className="strategy-list">
                  {automaton.strategies.map((strategy) => (
                    <li key={`${strategy.key.protocol}-${strategy.key.templateId}`}>
                      <strong>{strategy.key.templateId}</strong>
                      <span>{strategy.key.protocol} · {strategy.status}</span>
                    </li>
                  ))}
                </ul>
              ) : <p className="empty-copy">No indexed strategies are available.</p>}
            </section>
          ) : null}
        </div>
      </div>
    </aside>
  );
}
