import type { AutomatonDetail } from "@ic-automaton/shared";
import { useCommandSession } from "../../hooks/useCommandSession";
import type { WalletSession } from "../../wallet/useWalletSession";

interface CommandLinePanelProps {
  automaton: AutomatonDetail | null;
  canExecute: boolean;
  errorMessage: string | null;
  isLoading: boolean;
  selectedCanisterId: string | null;
  viewerAddress: string | null;
  walletSession: WalletSession | null;
  enabled?: boolean;
}

export function CommandLinePanel({
  automaton,
  viewerAddress,
  walletSession,
  enabled = true
}: CommandLinePanelProps) {
  const session = useCommandSession({
    automaton,
    viewerAddress
  }, walletSession, enabled);

  return (
    <section className="cli-section" aria-labelledby="command-line-heading">
      <div className="panel-heading">
        <h3 id="command-line-heading">Command Surface</h3>
        <span className="panel-note">Interactive terminal</span>
      </div>

      <div className="cli-output" aria-live="polite" role="log">
        {session.entries.map((entry) => (
          <div className={`cli-line is-${entry.kind}`} key={entry.id}>
            {entry.text}
          </div>
        ))}
      </div>

      <form
        className="cli-input-row"
        onSubmit={(event) => {
          event.preventDefault();
          void session.submitInput();
        }}
      >
        <span className="cli-prompt-label">{"\u003e"}</span>
        <input
          aria-label="Terminal command"
          className="cli-input"
          onChange={(event) => {
            session.setInputValue(event.currentTarget.value);
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowUp") {
              event.preventDefault();
              session.stepHistory("up");
            }

            if (event.key === "ArrowDown") {
              event.preventDefault();
              session.stepHistory("down");
            }

            if (event.key === "Escape") {
              session.clearOutput();
            }
          }}
          placeholder="Type a command and press Enter"
          value={session.inputValue}
        />
        <button className="cli-send" disabled={!session.canSubmit || session.isSubmitting} type="submit">
          {session.isSubmitting ? "SENDING..." : "SEND"}
        </button>
      </form>

      <p className="cli-readonly">
        {viewerAddress === null
          ? "Connect a wallet to use protected commands."
          : "Wallet is connected. Protected commands are available when the terminal allows them."}
      </p>
    </section>
  );
}
