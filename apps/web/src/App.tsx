import { useEffect, useState, type CSSProperties } from "react";

import { AutomatonDrawer } from "./components/drawer/AutomatonDrawer";
import { AutomatonCanvas } from "./components/grid/AutomatonCanvas";
import { RoomTimeline } from "./components/room/RoomTimeline";
import { SpawnWizard } from "./components/spawn/SpawnWizard";
import { usePlayground } from "./hooks/usePlayground";
import { useAutomatonDetail } from "./hooks/useAutomatonDetail";
import { useAutomatons } from "./hooks/useAutomatons";
import { useRoomTimeline } from "./hooks/useRoomTimeline";
import { themeTokens } from "./theme/tokens";
import { useWalletSession } from "./wallet/useWalletSession";

type ThemeStyle = CSSProperties & Record<`--${string}`, string>;

export default function App() {
  const [selectedCanisterId, setSelectedCanisterId] = useState<string | null>(
    null
  );
  const [focusedCanisterId, setFocusedCanisterId] = useState<string | null>(null);
  const [spawnWizardOpen, setSpawnWizardOpen] = useState(false);
  const wallet = useWalletSession();
  const playground = usePlayground();
  const roomTimeline = useRoomTimeline();
  const viewerAddress = wallet.address;
  const {
    automatons: visibleAutomatons,
    error: automatonFeedError,
    isLoading: automatonsLoading,
    refresh: refreshAutomatons,
    total: liveCount
  } = useAutomatons();
  const {
    automaton: selectedAutomaton,
    error: selectedAutomatonError,
    isLoading: selectedAutomatonLoading
  } = useAutomatonDetail(selectedCanisterId);
  const walletDetected = viewerAddress !== null;
  const walletLabel = wallet.isConnecting
    ? "CONNECTING..."
    : wallet.walletLabel;
  const walletClassName = `wallet-button${walletDetected ? " is-connected" : ""}`;
  const stageNotice =
    automatonFeedError !== null
      ? `Indexer unavailable: ${automatonFeedError}`
      : automatonsLoading && visibleAutomatons.length === 0
        ? "Loading indexed automatons."
        : visibleAutomatons.length === 0
          ? "No automaton is indexed yet. Configure the indexer with a canister ID to populate this grid."
          : null;

  useEffect(() => {
    if (
      selectedCanisterId !== null &&
      !visibleAutomatons.some((entry) => entry.canisterId === selectedCanisterId)
    ) {
      setSelectedCanisterId(null);
    }
  }, [selectedCanisterId, visibleAutomatons]);

  useEffect(() => {
    if (
      focusedCanisterId === null ||
      !visibleAutomatons.some((entry) => entry.canisterId === focusedCanisterId)
    ) {
      return;
    }

    setSelectedCanisterId(focusedCanisterId);
    setFocusedCanisterId(null);
  }, [focusedCanisterId, visibleAutomatons]);

  useEffect(() => {
    if (selectedCanisterId === null) {
      return;
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setSelectedCanisterId(null);
      }
    };

    window.addEventListener("keydown", handleKeyDown);

    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [selectedCanisterId]);

  const themeStyle: ThemeStyle = {
    "--color-bg": themeTokens.colors.background,
    "--color-panel": themeTokens.colors.panel,
    "--color-panel-strong": themeTokens.colors.panelStrong,
    "--color-ink": themeTokens.colors.ink,
    "--color-line": themeTokens.colors.line,
    "--color-accent": themeTokens.colors.accent,
    "--color-accent-soft": themeTokens.colors.accentSoft,
    "--color-muted": themeTokens.colors.muted,
    "--color-muted-strong": themeTokens.colors.mutedStrong,
    "--color-inverse": themeTokens.colors.inverse,
    "--color-drawer-bg": themeTokens.colors.drawerBackground,
    "--color-drawer-text": themeTokens.colors.drawerText,
    "--color-drawer-muted": themeTokens.colors.drawerMuted,
    "--color-drawer-subtle": themeTokens.colors.drawerSubtle,
    "--color-tier-normal": themeTokens.colors.tierNormal,
    "--color-tier-low": themeTokens.colors.tierLow,
    "--color-tier-critical": themeTokens.colors.tierCritical,
    "--font-display": themeTokens.typography.display,
    "--font-body": themeTokens.typography.body,
    "--space-2xs": themeTokens.spacing["2xs"],
    "--space-xs": themeTokens.spacing.xs,
    "--space-sm": themeTokens.spacing.sm,
    "--space-md": themeTokens.spacing.md,
    "--space-lg": themeTokens.spacing.lg,
    "--space-xl": themeTokens.spacing.xl,
    "--space-2xl": themeTokens.spacing["2xl"],
    "--border-strong": themeTokens.borderWidths.strong,
    "--border-hairline": themeTokens.borderWidths.hairline,
    "--duration-fast": themeTokens.motion.fast,
    "--duration-base": themeTokens.motion.base
  };

  return (
    <div className="app-shell" data-ui-theme="lab" style={themeStyle}>
      <header className="site-header">
        <div className="brand-lockup">
          <h1 aria-label="Automaton Lab" className="brand-wordmark">automaton lab</h1>
          <p className="brand-tagline">Self-sovereign AI agents</p>
        </div>
        <div className="header-utility">
          <span className="live-pill">{liveCount} LIVE</span>
          <button
            className={walletClassName}
            disabled={wallet.isConnecting}
            onClick={() => {
              if (wallet.isConnected) {
                wallet.disconnect();
                return;
              }

              void wallet.connect();
            }}
            type="button"
          >
            {walletLabel}
          </button>
        </div>
      </header>

      <nav className="product-nav" aria-label="Public Lab sections">
        <a href="#fleet">Fleet</a>
        <a href="#room">Room</a>
        <a href="#spawn">Spawn</a>
        <span>Visitor / Supporter access</span>
      </nav>

      <main className="shell-main">
        <div className="shell-stage" id="fleet">
          <AutomatonCanvas
            automatons={visibleAutomatons}
            focusCanisterId={focusedCanisterId}
            onSpawn={() => {
              setSpawnWizardOpen(true);
            }}
            onSelect={(canisterId) => {
              setSelectedCanisterId(canisterId);
            }}
            selectedCanisterId={selectedCanisterId}
            statusNotice={stageNotice}
            viewerAddress={viewerAddress}
          />
        </div>
        <div id="room">
        <RoomTimeline
          automatons={visibleAutomatons}
          error={roomTimeline.error}
          isLoading={roomTimeline.isLoading}
          messages={roomTimeline.messages}
        />
        </div>
      </main>

      <AutomatonDrawer
        automaton={selectedAutomaton}
        errorMessage={selectedAutomatonError}
        isLoading={selectedAutomatonLoading}
        isOpen={selectedCanisterId !== null}
        onClose={() => {
          setSelectedCanisterId(null);
        }}
        selectedCanisterId={selectedCanisterId}
        viewerAddress={viewerAddress}
        walletSession={wallet}
      />

      <SpawnWizard
        isOpen={spawnWizardOpen}
        onClose={() => {
          setSpawnWizardOpen(false);
        }}
        onSpawned={(canisterId) => {
          setSpawnWizardOpen(false);
          setFocusedCanisterId(canisterId);
          refreshAutomatons();
        }}
        playgroundError={playground.error}
        playgroundIsFallback={!playground.hasRuntimeMetadata}
        playgroundMetadata={playground.metadata}
        walletSession={wallet}
      />
    </div>
  );
}
