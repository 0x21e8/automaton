import { useEffect, useMemo, useState } from "react";
import type { AutomatonDetail } from "@ic-automaton/shared";

function renderedDiff(parent: string, child: string): string[] {
  const left = parent.split(/\s+/);
  const right = child.split(/\s+/);
  const length = Math.max(left.length, right.length);
  const lines: string[] = [];
  for (let index = 0; index < length; index += 1) {
    if (left[index] === right[index]) continue;
    if (left[index] !== undefined) lines.push(`− ${left[index]}`);
    if (right[index] !== undefined) lines.push(`+ ${right[index]}`);
    if (lines.length >= 80) break;
  }
  return lines;
}

export function AncestryPanel({ automaton }: { automaton: AutomatonDetail }) {
  const [parent, setParent] = useState<AutomatonDetail | null>(null);
  useEffect(() => {
    setParent(null);
    if (automaton.parentId === null) return;
    const controller = new AbortController();
    void fetch(`/api/automatons/${encodeURIComponent(automaton.parentId)}`, { signal: controller.signal })
      .then(async (response) => response.ok ? response.json() as Promise<AutomatonDetail> : null)
      .then(setParent)
      .catch(() => undefined);
    return () => controller.abort();
  }, [automaton.parentId]);

  const verified = parent !== null &&
    parent.constitutionVerification.status === "verified" &&
    automaton.constitutionVerification.status === "verified" &&
    parent.constitutionHash !== null &&
    automaton.constitutionHash !== null &&
    parent.constitutionHash === automaton.parentConstitutionHash;
  const diff = useMemo(() => verified && parent?.constitution && automaton.constitution
    ? renderedDiff(parent.constitution, automaton.constitution)
    : [], [automaton.constitution, parent, verified]);

  return <section className="detail-field" aria-labelledby="ancestry-heading">
    <div className="lbl" id="ancestry-heading">Ancestry</div>
    <p>Generation {automaton.generation ?? 0}</p>
    <p>Parent: {automaton.parentId ?? "founder"}</p>
    <p>Children: {automaton.childIds.length === 0 ? "none" : automaton.childIds.join(", ")}</p>
    {automaton.parentId !== null ? verified ? <details>
      <summary>Verified constitutional drift</summary>
      <pre>{diff.length === 0 ? "No lexical drift." : diff.join("\n")}</pre>
      <small>Crude word-position diff; both public documents were SHA-256 verified.</small>
    </details> : <p role="status">Parent diff withheld until the public parent constitution matches the factory hash.</p> : null}
  </section>;
}
