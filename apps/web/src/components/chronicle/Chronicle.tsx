import type { ChronicleDay } from "@ic-automaton/shared";

export function Chronicle({ days, error }: { days: ChronicleDay[]; error: string | null }) {
  return <section aria-label="World chronicle" className="chronicle">
    <p className="section-label">Observatory record</p>
    <h2>Chronicle</h2>
    <p>Factual indexer labels with source links. The observatory records; it does not endorse.</p>
    {error ? <p>{error}</p> : null}
    {days.map((day) => <article key={day.date}>
      <h3>{day.date}</h3>
      {day.entries.length === 0 ? <p>No recorded events.</p> : <ol>{day.entries.map((entry) => <li key={entry.id}>
        <time dateTime={new Date(entry.timestamp).toISOString()}>{new Date(entry.timestamp).toLocaleTimeString()}</time>{" "}
        <strong>{entry.headline}</strong> — {entry.detail}{" "}
        {entry.provenance.map((source) => <a href={source.href} key={`${entry.id}:${source.href}`} rel="noreferrer">[{source.label}]</a>)}
      </li>)}</ol>}
    </article>)}
  </section>;
}
