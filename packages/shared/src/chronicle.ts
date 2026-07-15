export type ChronicleEntryKind = "birth" | "death" | "deal" | "runway_crisis" | "room_activity" | "journal";

export interface ChronicleProvenance {
  label: string;
  href: string;
}

export interface ChronicleEntry {
  id: string;
  kind: ChronicleEntryKind;
  timestamp: number;
  headline: string;
  detail: string;
  canisterIds: string[];
  provenance: ChronicleProvenance[];
}

export interface ChronicleDay {
  date: string;
  generatedAt: number;
  entries: ChronicleEntry[];
  population?: {
    living: number;
    births: number;
    deaths: number;
    medianRunwaySeconds: number | null;
    patronageUsdcRawPerLiving: string;
    positiveInflowUsdcRawPerLiving: string;
  };
}

export interface ChronicleFeed {
  days: ChronicleDay[];
  nextBefore: number | null;
}
