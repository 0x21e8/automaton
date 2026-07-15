import type { AutomatonDetail, ChronicleFeed } from "@ic-automaton/shared";

function tokens(value: string): Set<string> {
  return new Set(value.toLowerCase().match(/[\p{L}\p{N}]+/gu) ?? []);
}

/**
 * Cheap lexical Jaccard dispersion for small narrative-heredity populations.
 * This is intentionally labelled crude: it measures word-set difference,
 * not semantic or population-genetic diversity.
 */
export function constitutionalDiversity(automatons: AutomatonDetail[]): number | null {
  const sets = automatons
    .filter((automaton) =>
      !automaton.metabolism?.diedAt &&
      automaton.constitution &&
      automaton.constitutionHash &&
      automaton.constitutionVerification.status === "verified"
    )
    .map((automaton) => tokens(automaton.constitution!));
  if (sets.length < 2) return null;
  let total = 0;
  let pairs = 0;
  for (let left = 0; left < sets.length; left += 1) {
    for (let right = left + 1; right < sets.length; right += 1) {
      const union = new Set([...sets[left]!, ...sets[right]!]);
      const intersection = [...sets[left]!].filter((token) => sets[right]!.has(token)).length;
      total += union.size === 0 ? 0 : 1 - intersection / union.size;
      pairs += 1;
    }
  }
  return Number((total / pairs).toFixed(6));
}

export function buildFitnessObservatory(
  automatons: AutomatonDetail[],
  chronicle: ChronicleFeed | null
) {
  const byId = new Map(automatons.map((automaton) => [automaton.canisterId, automaton]));
  const descendants = automatons.filter((automaton) => automaton.parentId !== null);
  const outlivedParent = descendants.filter((child) => {
    const parent = child.parentId ? byId.get(child.parentId) : undefined;
    return parent?.metabolism?.diedAt !== undefined && parent.metabolism.diedAt !== null &&
      (child.metabolism?.diedAt === undefined || child.metabolism.diedAt === null || child.metabolism.diedAt > parent.metabolism.diedAt);
  }).length;
  return {
    framing: "narrative heredity; populations in the tens are not population genetics",
    constitutionalDiversity: constitutionalDiversity(automatons),
    constitutionalDiversityMethod: "crude lexical Jaccard dispersion over living, hash-verified public constitutions",
    lineage: {
      descendants: descendants.length,
      maxGeneration: automatons.reduce((max, automaton) => Math.max(max, automaton.generation ?? 0), 0),
      descendantsThatOutlivedParent: outlivedParent
    },
    population: chronicle?.days[0]?.population ?? null
  };
}
