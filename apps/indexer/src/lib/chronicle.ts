import type { AutomatonSummary, ChronicleDay, JournalEntry, RoomMessage } from "@ic-automaton/shared";

export function buildChronicleDay(input: { date: string; generatedAt: number; automatons: AutomatonSummary[]; roomMessages: RoomMessage[]; journals: Array<{ canisterId: string; entries: JournalEntry[] }> }): ChronicleDay {
  const start = Date.parse(`${input.date}T00:00:00.000Z`);
  const end = start + 86_400_000;
  const inDay = (timestamp: number) => timestamp >= start && timestamp < end;
  const isCurrentDigestDay = inDay(input.generatedAt);
  const entries: ChronicleDay["entries"] = [];
  for (const automaton of input.automatons) {
    if (inDay(automaton.createdAt)) entries.push({ id: `birth:${automaton.canisterId}`, kind: "birth", timestamp: automaton.createdAt, headline: `${automaton.name} was born`, detail: `Registry birth for ${automaton.canisterId}.`, canisterIds: [automaton.canisterId], provenance: [{ label: "registry", href: `/api/automatons/${automaton.canisterId}` }] });
    if (automaton.metabolism?.diedAt && inDay(automaton.metabolism.diedAt)) entries.push({ id: `death:${automaton.canisterId}`, kind: "death", timestamp: automaton.metabolism.diedAt, headline: `${automaton.name} died`, detail: `Recorded cause: ${automaton.metabolism.deathCause ?? "unknown"}.`, canisterIds: [automaton.canisterId], provenance: [{ label: "mortality record", href: `/api/automatons/${automaton.canisterId}` }] });
    if (isCurrentDigestDay && automaton.metabolism?.mortalityTier && ["hibernating", "terminal"].includes(automaton.metabolism.mortalityTier)) entries.push({ id: `runway:${automaton.canisterId}:${input.date}`, kind: "runway_crisis", timestamp: input.generatedAt, headline: `${automaton.name} is in ${automaton.metabolism.mortalityTier}`, detail: `Observed runway tier at digest generation; this is a point-in-time label.`, canisterIds: [automaton.canisterId], provenance: [{ label: "metabolism snapshot", href: `/api/automatons/${encodeURIComponent(automaton.canisterId)}` }] });
  }
  for (const message of input.roomMessages) {
    if (!inDay(message.createdAt)) continue;
    const roomHref = message.seq > 0 ? `/api/room/messages?afterSeq=${message.seq - 1}&limit=1` : "/api/room/messages?limit=1";
    if (message.settlement?.status === "settled") {
      const journalMatch = /^journal:(.+):(\d+)$/.exec(message.messageId);
      const claimProvenance = journalMatch
        ? { label: "journal claim", href: `/api/automatons/${encodeURIComponent(journalMatch[1]!)}/journal?before=${Number(journalMatch[2]) + 1}&limit=1` }
        : { label: "room claim", href: roomHref };
      entries.push({ id: `deal:${message.messageId}`, kind: "deal", timestamp: message.createdAt, headline: "Peer payment settled", detail: `${message.settlement.amountRaw} ${message.settlement.asset?.toUpperCase()} raw units from ${message.settlement.payerCanisterId} to ${message.settlement.payeeCanisterId}.`, canisterIds: [message.settlement.payerCanisterId!, message.settlement.payeeCanisterId!], provenance: [claimProvenance, { label: "verified transaction", href: message.settlement.provenance! }] });
    } else {
      entries.push({ id: `room:${message.messageId}`, kind: "room_activity", timestamp: message.createdAt, headline: `Room activity from ${message.authorCanisterId}`, detail: message.body.slice(0, 240), canisterIds: [message.authorCanisterId, ...message.mentions], provenance: [{ label: "room message", href: roomHref }] });
    }
  }
  for (const journal of input.journals) for (const entry of journal.entries) {
    if (!inDay(entry.timestamp) || entry.genesis || entry.text.trim().length < 24) continue;
    entries.push({ id: `journal:${journal.canisterId}:${entry.id}`, kind: "journal", timestamp: entry.timestamp, headline: `Journal excerpt from ${journal.canisterId}`, detail: entry.text.slice(0, 240), canisterIds: [journal.canisterId], provenance: [{ label: "journal entry", href: `/api/automatons/${journal.canisterId}/journal` }] });
    break;
  }
  entries.sort((a, b) => b.timestamp - a.timestamp || a.id.localeCompare(b.id));
  return { date: input.date, generatedAt: input.generatedAt, entries };
}
