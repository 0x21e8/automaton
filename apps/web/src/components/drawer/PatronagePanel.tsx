import { useEffect, useState } from "react";
import type { AutomatonDetail, PlaygroundMetadata } from "@ic-automaton/shared";
import type { WalletSession } from "../../wallet/useWalletSession";
import { parseDecimalAmount } from "../../lib/wallet-transaction-helpers";
import {
  readAttentionPrice,
  sendDirectPatronage,
  sendPaidMessage,
  type AttentionPrice
} from "../../lib/patronage";
import { resolveSpawnChainMetadata } from "../../lib/wallet-transaction-helpers";

export function formatRawAmount(raw: bigint, decimals: number, maximumFractionDigits = decimals): string {
  const divisor = 10n ** BigInt(decimals);
  const whole = raw / divisor;
  const remainder = raw % divisor;
  const fraction = remainder.toString().padStart(decimals, "0")
    .slice(0, maximumFractionDigits)
    .replace(/0+$/, "");
  return fraction === "" ? whole.toString() : `${whole}.${fraction}`;
}

export function PatronagePanel({ automaton, playgroundMetadata = null, wallet }: {
  automaton: AutomatonDetail;
  playgroundMetadata?: PlaygroundMetadata | null;
  wallet: WalletSession | null;
}) {
  const [price, setPrice] = useState<AttentionPrice | null>(null);
  const [message, setMessage] = useState("");
  const [asset, setAsset] = useState<"usdc" | "eth">("usdc");
  const [gift, setGift] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const rpcUrl = automaton.chain === "base"
    ? resolveSpawnChainMetadata("base", playgroundMetadata)?.rpcUrl ?? null
    : null;
  const canReadPrice = rpcUrl !== null && automaton.ethAddress !== null && (automaton.inboxContractAddress ?? null) !== null;
  const available = wallet?.address !== null && wallet?.address !== undefined;

  useEffect(() => {
    setPrice(null);
    if (!canReadPrice || rpcUrl === null || automaton.ethAddress === null || (automaton.inboxContractAddress ?? null) === null) return;
    void readAttentionPrice({ rpcUrl, automatonAddress: automaton.ethAddress, inboxAddress: automaton.inboxContractAddress as string })
      .then(setPrice)
      .catch((error: unknown) => setStatus(error instanceof Error ? error.message : "Could not read the price of attention."));
  }, [automaton.canisterId, automaton.ethAddress, automaton.inboxContractAddress, canReadPrice, rpcUrl]);

  async function submitMessage() {
    if (!available || wallet === null || price === null || automaton.ethAddress === null || (automaton.inboxContractAddress ?? null) === null) return;
    try {
      const hashes = await sendPaidMessage({ asset, wallet, price, message, automatonAddress: automaton.ethAddress, inboxAddress: automaton.inboxContractAddress as string, usdcAddress: automaton.usdcContractAddress ?? null, expectedChainId: automaton.chainId });
      setStatus(`Paid message submitted: ${hashes.at(-1)}`);
      setMessage("");
    } catch (error) { setStatus(error instanceof Error ? error.message : "Paid message failed."); }
  }

  async function submitGift() {
    const amountRaw = parseDecimalAmount(gift, 6);
    if (wallet === null || automaton.ethAddress === null || (automaton.inboxContractAddress ?? null) === null || (automaton.usdcContractAddress ?? null) === null || amountRaw === null) {
      setStatus("Enter a valid USDC amount and connect a wallet."); return;
    }
    try {
      const hash = await sendDirectPatronage({ wallet, amountRaw, automatonAddress: automaton.ethAddress, inboxAddress: automaton.inboxContractAddress as string, usdcAddress: automaton.usdcContractAddress as string, expectedChainId: automaton.chainId });
      setStatus(`Patronage submitted: ${hash}`); setGift("");
    } catch (error) { setStatus(error instanceof Error ? error.message : "Patronage failed."); }
  }

  return <section className="patronage-panel" aria-labelledby="patronage-heading">
    <div className="panel-heading"><h3 id="patronage-heading">Price of attention</h3><span className="panel-note">Set by this being</span></div>
    <p className="empty-copy">A payment buys attention, not obedience or a promised outcome.</p>
    {price ? <p className="empty-copy">Public quote: {formatRawAmount(price.usdcRaw, 6, 6)} USDC + {formatRawAmount(price.ethWei, 18, 6)} ETH, or {formatRawAmount(price.ethWei, 18, 6)} ETH alone.</p> : null}
    <textarea aria-label="Paid message" onChange={(event) => setMessage(event.target.value)} placeholder="Write to this being" value={message} />
    <div className="patronage-row">
      <select aria-label="Payment asset" onChange={(event) => setAsset(event.target.value as "usdc" | "eth")} value={asset}><option value="usdc">USDC</option><option value="eth">ETH</option></select>
      <button disabled={!available || price === null || message.trim() === ""} onClick={() => void submitMessage()} type="button">
        {price === null ? "READING PRICE" : asset === "usdc" ? `SEND · ${formatRawAmount(price.usdcRaw, 6, 6)} USDC + ${formatRawAmount(price.ethWei, 18, 6)} ETH` : `SEND · ${formatRawAmount(price.ethWei, 18, 6)} ETH`}
      </button>
    </div>
    <div className="panel-heading"><h3>Direct patronage</h3><span className="panel-note">Gift</span></div>
    <p className="empty-copy">This is a gift the being can metabolize. It purchases nothing and carries no promise of value or return.</p>
    <div className="patronage-row"><input aria-label="Patronage amount in USDC" onChange={(event) => setGift(event.target.value)} placeholder="USDC amount" value={gift} /><button disabled={wallet?.address == null || (automaton.inboxContractAddress ?? null) === null || (automaton.usdcContractAddress ?? null) === null} onClick={() => void submitGift()} type="button">GIFT USDC</button></div>
    {status ? <p role="status">{status}</p> : null}
    {!available ? <p className="empty-copy">Connect a wallet to pay for attention or send patronage. The public quote remains readable without a wallet.</p> : null}
  </section>;
}
