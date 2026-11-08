// Mirrors the Rust structs in src-tauri/modcore (serde camelCase).

export type ModKind = "skin" | "sound" | "ui" | "other";

export interface ModEntry {
  id: string;
  name: string;
  kind: ModKind;
  hero: string;
  notes: string;
  version: string;
  storedFile: string;
  originalName: string;
  size: number;
  stamp: string;
  addedAt: string;
  updatedAt: string;
  enabled: boolean;
  verifiedBuild: string | null;
  files: string[];
}

export interface Profile {
  id: string;
  name: string;
  mods: string[];
}

export interface SafeModeEvent {
  fromBuild: string;
  toBuild: string;
  at: string;
  disabled: string[];
}

export type Action =
  | { kind: "createAddonsDir" }
  | { kind: "remove"; file: string; modName: string | null }
  | { kind: "copy"; file: string; modId: string; modName: string }
  | { kind: "addSearchPath" }
  | { kind: "removeSearchPath" };

export interface Plan {
  actions: Action[];
  warnings: string[];
  blockers: string[];
  layout: [string, string][];
}

export interface Conflict {
  path: string;
  mods: string[];
}

export interface GameStatus {
  dir: string | null;
  found: boolean;
  buildId: string | null;
  running: boolean;
  addonsLine: boolean | null;
}

export interface Overview {
  mods: ModEntry[];
  profiles: Profile[];
  settings: { gameDir: string | null };
  safeMode: SafeModeEvent | null;
  game: GameStatus;
  plan: Plan;
  conflicts: Conflict[];
  dataDir: string;
}

export interface ApplyReport {
  done: string[];
  backup: string | null;
}

export interface ModMeta {
  name?: string;
  kind?: ModKind;
  hero?: string;
  notes?: string;
  version?: string;
}
