import type {
  AutomatonRecord,
  JournalEntry,
  MonologueEntry
} from "./automaton.js";
import type { RoomMessage } from "./room.js";
import type {
  SessionAuditEntry,
  SpawnSession
} from "./spawn.js";

export const AUTOMATON_EVENT_TYPES = [
  "spawn",
  "update",
  "action",
  "message",
  "monologue",
  "journal",
  "offline"
] as const;

export const SPAWN_EVENT_TYPES = [
  "spawn.session.updated",
  "spawn.session.completed",
  "spawn.session.failed",
  "spawn.session.expired"
] as const;

export type AutomatonEventType = (typeof AUTOMATON_EVENT_TYPES)[number];
export type SpawnEventType = (typeof SPAWN_EVENT_TYPES)[number];

export interface AutomatonSpawnEvent {
  type: "spawn";
  automaton: AutomatonRecord;
}

export interface AutomatonUpdateEvent {
  type: "update";
  canisterId: string;
  changes: Partial<AutomatonRecord>;
  timestamp: number;
}

export interface AutomatonActionEvent {
  type: "action";
  canisterId: string;
  action: string;
  turnId: string | null;
  timestamp: number;
}

export interface AutomatonMessageEvent {
  type: "message";
  message: RoomMessage;
}

export interface AutomatonMonologueEvent {
  type: "monologue";
  canisterId: string;
  entry: MonologueEntry;
}

export interface AutomatonJournalEvent {
  type: "journal";
  canisterId: string;
  entry: JournalEntry;
}

export interface AutomatonOfflineEvent {
  type: "offline";
  canisterId: string;
  timestamp: number;
}

export interface SpawnSessionUpdatedEvent {
  type: "spawn.session.updated";
  session: SpawnSession;
  audit: SessionAuditEntry[];
}

export interface SpawnSessionCompletedEvent {
  type: "spawn.session.completed";
  session: SpawnSession;
  audit: SessionAuditEntry[];
}

export interface SpawnSessionFailedEvent {
  type: "spawn.session.failed";
  session: SpawnSession;
  audit: SessionAuditEntry[];
}

export interface SpawnSessionExpiredEvent {
  type: "spawn.session.expired";
  session: SpawnSession;
  audit: SessionAuditEntry[];
}

export type RealtimeEvent =
  | AutomatonSpawnEvent
  | AutomatonUpdateEvent
  | AutomatonActionEvent
  | AutomatonMessageEvent
  | AutomatonMonologueEvent
  | AutomatonJournalEvent
  | AutomatonOfflineEvent
  | SpawnSessionUpdatedEvent
  | SpawnSessionCompletedEvent
  | SpawnSessionFailedEvent
  | SpawnSessionExpiredEvent;
