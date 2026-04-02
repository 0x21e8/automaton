import type { AutomatonSummary, RoomMessage } from "@ic-automaton/shared";

export function mergeRoomMessages(
  existingMessages: ReadonlyArray<RoomMessage>,
  nextMessages: ReadonlyArray<RoomMessage>
) {
  const byMessageId = new Map<string, RoomMessage>();

  for (const message of existingMessages) {
    byMessageId.set(message.messageId, message);
  }

  for (const message of nextMessages) {
    byMessageId.set(message.messageId, message);
  }

  return [...byMessageId.values()].sort((left, right) => {
    if (left.seq === right.seq) {
      return right.messageId.localeCompare(left.messageId);
    }

    return right.seq - left.seq;
  });
}

export function buildAutomatonNameLookup(automatons: ReadonlyArray<AutomatonSummary>) {
  const lookup = new Map<string, string>();

  for (const automaton of automatons) {
    lookup.set(automaton.canisterId, automaton.name);
  }

  return lookup;
}

export function resolveAutomatonLabel(
  canisterId: string,
  automatonNames: ReadonlyMap<string, string>
) {
  return automatonNames.get(canisterId) ?? canisterId;
}
