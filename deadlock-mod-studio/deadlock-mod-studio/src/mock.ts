// In-memory stand-in for the Tauri backend, used only when the UI runs in a
// plain browser (`npm run dev`) so the design can be worked on without the
// desktop app. Nothing here touches real files.

import type { Action, ModEntry, Overview, Profile } from "./types";
import type { Api } from "./api";

const BUILD = "20614033";
const now = () => new Date().toISOString();

const mk = (id: string, name: string, kind: ModEntry["kind"], hero: string, files: string[], extra: Partial<ModEntry> = {}): ModEntry => ({
  id,
  name,
  kind,
  hero,
  notes: "",
  version: "1",
  storedFile: `${id}.vpk`,
  originalName: `${name.toLowerCase().replace(/\s+/g, "_")}.vpk`,
  size: 180_000 + files.length * 41_000,
  stamp: id,
  addedAt: "2026-11-08T12:00:00+00:00",
  updatedAt: "2026-11-08T12:00:00+00:00",
  enabled: false,
  verifiedBuild: null,
  files,
  ...extra,
});

const storm = [
  "sounds/abilities/gigawatt/storm_cloud_cast.vsnd_c",
  "sounds/abilities/gigawatt/storm_cloud_lp.vsnd_c",
  "sounds/abilities/gigawatt/storm_cloud_aoe_lp.vsnd_c",
  "sounds/abilities/gigawatt/storm_cloud_bolt_cast.vsnd_c",
  "sounds/abilities/gigawatt/storm_cloud_end.vsnd_c",
];

let mods: ModEntry[] = [
  mk("a1", "Seven Storm Cloud remix", "sound", "Seven", storm, { enabled: true, verifiedBuild: BUILD, version: "3", notes: "Ult loops re-cut to 4 s. Same loudness as the originals." }),
  mk("a2", "Shop music: late-night jazz", "sound", "", ["sounds/music/shop_music_01.vsnd_c"], { enabled: true, verifiedBuild: "20590112" }),
  mk("a3", "Storm Cloud bolts (alt take)", "sound", "Seven", [storm[3], "sounds/abilities/gigawatt/storm_cloud_bolt_explo_01.vsnd_c"], { enabled: true }),
  mk("a4", "Brass HUD frame", "ui", "", ["panorama/images/hud/frame_psd.vtex_c"]),
];
let profiles: Profile[] = [{ id: "p1", name: "Full", mods: ["a1", "a2", "a3"] }];
let applied: string[] = ["a1", "a2"];
let log: string[] = [`${now().slice(0, 19)}+00:00  Imported "Seven Storm Cloud remix" (5 files inside)`];

function plan() {
  const want = mods.filter((m) => m.enabled).map((m) => m.id);
  const actions: Action[] = [];
  want.forEach((id, i) => {
    if (applied[i] !== id) {
      const file = `pak${String(i + 1).padStart(2, "0")}_dir.vpk`;
      if (applied[i]) actions.push({ kind: "remove", file, modName: mods.find((m) => m.id === applied[i])?.name ?? null });
      actions.push({ kind: "copy", file, modId: id, modName: mods.find((m) => m.id === id)!.name });
    }
  });
  for (let i = want.length; i < applied.length; i++) {
    actions.push({ kind: "remove", file: `pak${String(i + 1).padStart(2, "0")}_dir.vpk`, modName: mods.find((m) => m.id === applied[i])?.name ?? null });
  }
  return {
    actions,
    warnings: conflicts().length ? [`${conflicts().length} internal file(s) are changed by more than one enabled mod`] : [],
    blockers: [],
    layout: want.map((id, i) => [`pak${String(i + 1).padStart(2, "0")}_dir.vpk`, mods.find((m) => m.id === id)!.name] as [string, string]),
  };
}

function conflicts() {
  const map = new Map<string, string[]>();
  for (const m of mods.filter((x) => x.enabled)) for (const f of m.files) map.set(f, [...(map.get(f) ?? []), m.id]);
  return [...map].filter(([, v]) => v.length > 1).map(([path, ids]) => ({ path, mods: ids }));
}

const ok = async () => undefined;

export const mockApi: Api = {
  overview: async (): Promise<Overview> => ({
    mods: structuredClone(mods),
    profiles: structuredClone(profiles),
    settings: { gameDir: "D:\\Games\\SteamLibrary\\steamapps\\common\\Deadlock" },
    safeMode: null,
    game: { dir: "D:\\Games\\SteamLibrary\\steamapps\\common\\Deadlock", found: true, buildId: BUILD, running: false, addonsLine: true },
    plan: plan(),
    conflicts: conflicts(),
    dataDir: "C:\\Users\\you\\AppData\\Roaming\\app.deadlockmodstudio.local",
  }),
  checkForUpdate: async () => null,
  dismissSafeMode: ok,
  detectGameDir: async () => "D:\\Games\\SteamLibrary\\steamapps\\common\\Deadlock",
  setGameDir: ok,
  importMods: async () => [],
  replaceModFile: ok,
  updateMod: async (id, meta) => {
    mods = mods.map((m) => (m.id === id ? { ...m, ...Object.fromEntries(Object.entries(meta).filter(([, v]) => v !== undefined)) } : m));
  },
  removeMod: async (id) => {
    mods = mods.filter((m) => m.id !== id);
  },
  setEnabled: async (id, enabled) => {
    mods = mods.map((m) => (m.id === id ? { ...m, enabled } : m));
  },
  setAllEnabled: async (enabled) => {
    mods = mods.map((m) => ({ ...m, enabled }));
  },
  reorder: async (ids) => {
    mods = ids.map((id) => mods.find((m) => m.id === id)!);
  },
  markVerified: async (id, v) => {
    mods = mods.map((m) => (m.id === id ? { ...m, verifiedBuild: v ? BUILD : null } : m));
  },
  saveProfile: async (name) => {
    const p = { id: String(Date.now()), name, mods: mods.filter((m) => m.enabled).map((m) => m.id) };
    profiles = [...profiles, p];
    return p;
  },
  deleteProfile: async (id) => {
    profiles = profiles.filter((p) => p.id !== id);
  },
  applyProfile: async (id) => {
    const p = profiles.find((x) => x.id === id)!;
    const on = p.mods.map((mid) => mods.find((m) => m.id === mid)!).filter(Boolean).map((m) => ({ ...m, enabled: true }));
    mods = [...on, ...mods.filter((m) => !p.mods.includes(m.id)).map((m) => ({ ...m, enabled: false }))];
  },
  applyChanges: async () => {
    applied = mods.filter((m) => m.enabled).map((m) => m.id);
    log = [`${now().slice(0, 19)}+00:00  Applied changes`, ...log];
    return { done: [], backup: null };
  },
  restoreVanilla: async () => {
    mods = mods.map((m) => ({ ...m, enabled: false }));
    applied = [];
    return { done: [], backup: null };
  },
  readLog: async () => log,
  openFolder: ok,
  launchGame: ok,
};
