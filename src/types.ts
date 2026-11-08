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
  sharedAddonsLine: boolean | null;
}

export interface Overview {
  mods: ModEntry[];
  profiles: Profile[];
  settings: { gameDir: string | null; csdkDir: string | null; blenderExe: string | null; blenderAddon: string | null };
  safeMode: SafeModeEvent | null;
  game: GameStatus;
  plan: Plan;
  conflicts: Conflict[];
  dataDir: string;
  soundProjects: SoundProject[];
  csdkFound: boolean;
  modelProjects: ModelProject[];
  s2vInstalled: boolean;
  s2vVersion: string;
}

export interface SoundJob {
  target: string;
  source: string;
  volumeDb: number;
}

export interface VoiceMute {
  hero: string;
  slot: number;
}

export interface SoundUse {
  hero: string;
  heroName: string;
  category: string;
  slot: number | null;
  abilityName: string | null;
  part: string;
  event: string;
}

export interface AbilityInfo {
  slot: number;
  key: string;
  name: string;
  voiceLines: number;
}

export interface HeroInfo {
  code: string;
  name: string;
  abilities: AbilityInfo[];
}

export interface SoundProject {
  id: string;
  name: string;
  hero: string;
  items: SoundJob[];
  muteVoice: VoiceMute[];
  modId: string | null;
  lastBuilt: string | null;
}

export interface GameSound {
  path: string;
  size: number;
  looped: boolean;
  uses: SoundUse[];
}

export interface SoundSearch {
  total: number;
  results: GameSound[];
}

export interface Level {
  loudnessDb: number;
  peakDb: number;
  durationSecs: number;
  channels: number;
  rate: number;
}

export interface ItemReport {
  target: string;
  original: Level;
  yours: Level;
  result: Level;
  gainDb: number;
  volumeDb: number;
  limited: boolean;
  looped: boolean;
  convertedChannels: boolean;
}

export interface BuildResult {
  report: { items: ItemReport[]; vpk: string; vpkSize: number; log: string; muted: number };
  modId: string;
  modName: string;
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

export interface ModelHero {
  code: string;
  name: string;
  model: string;
}

export interface ModelProject {
  id: string;
  name: string;
  hero: string;
  model: string;
  created: string;
  extracted: string | null;
  extractedBuild: string | null;
  extractedTool: string | null;
  modId: string | null;
  lastBuilt: string | null;
}

export interface ModelSummary {
  bones: string[];
  meshes: number;
  lods: number;
  bodyGroups: string[];
  materials: string[];
  attachments: string[];
  hitboxes: number;
  ragdollBodies: number;
  clothNodes: number;
  animGraphs: string[];
  trianglesLod0: number;
}

export interface ExtractResult {
  report: { summary: ModelSummary; blenderFile: string; template: string; meshFiles: string[]; clothMissing: boolean; log: string };
  project: ModelProject;
}

export interface ModelBuildResult {
  report: { vpk: string; vpkSize: number; files: string[]; warnings: string[]; log: string };
  modId: string;
  modName: string;
}

export interface BlenderInstall {
  exe: string;
  version: string;
}

export interface Returned {
  folder: string;
  sent: string;
  blender: string;
  addon: string;
  glb: string;
  objects: { object: string; part: string; triangles: number }[];
  materials: string[];
}
