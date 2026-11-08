// In-memory stand-in for the Tauri backend, used only when the UI runs in a
// plain browser (`npm run dev`) so the design can be worked on without the
// desktop app. Nothing here touches real files.

import type { Action, ModEntry, Overview, Profile, SoundProject } from "./types";
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
let projects: SoundProject[] = [
  {
    id: "sp1",
    name: "Storm Cloud remix",
    hero: "Seven",
    items: [
      { target: "sounds/abilities/gigawatt/gigawatt_storm_cloud_lp.vsnd_c", source: "D:\\Audio\\storm_loop_v3.wav" },
      { target: "sounds/abilities/gigawatt/gigawatt_storm_cloud_cast.vsnd_c", source: "D:\\Audio\\thunder_cast.mp4" },
      { target: "sounds/abilities/gigawatt/a4_storm_cloud/impact_1.vsnd_c", source: "" },
    ],
    modId: "a1",
    lastBuilt: "2026-11-08T15:30:00+00:00",
  },
];
const gameSounds = [
  "a4_storm_cloud/end", "a4_storm_cloud/impact_1", "a4_storm_cloud/impact_2", "a4_storm_cloud/impact_3", "a4_storm_cloud/impact_4",
  "gigawatt_storm_cloud_aoe_lp", "gigawatt_storm_cloud_bolt_cast", "gigawatt_storm_cloud_bolt_explo_01", "gigawatt_storm_cloud_cast",
  "gigawatt_storm_cloud_end", "gigawatt_storm_cloud_lp", "gigawatt_power_surge_activate_lp", "gigawatt_static_charge_cast",
].map((s) => `sounds/abilities/gigawatt/${s}.vsnd_c`);
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
    settings: { gameDir: "D:\\Games\\SteamLibrary\\steamapps\\common\\Deadlock", csdkDir: "D:\\Desktop\\Reduced_CSDK_13" },
    safeMode: null,
    game: { dir: "D:\\Games\\SteamLibrary\\steamapps\\common\\Deadlock", found: true, buildId: BUILD, running: false, addonsLine: true, sharedAddonsLine: true },
    plan: plan(),
    conflicts: conflicts(),
    dataDir: "C:\\Users\\you\\AppData\\Roaming\\app.deadlockmodstudio.local",
    soundProjects: structuredClone(projects),
    csdkFound: true,
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
  searchGameSounds: async (query) => {
    const w = query.toLowerCase().split(/\s+/).filter(Boolean);
    const r = gameSounds.filter((p) => w.every((x) => p.includes(x)));
    return { total: r.length, results: r.map((path) => ({ path, size: 150000, looped: /_lp\./.test(path) })) };
  },
  audioLevel: async (kind, path) => ({
    loudnessDb: kind === "game" ? -15.8 : path.includes("mp4") ? -9.1 : -16.4,
    peakDb: kind === "game" ? -0.6 : -0.2,
    durationSecs: path.includes("_lp") || path.includes("loop") ? 18.02 : 5.56,
    channels: 2,
    rate: 44100,
  }),
  audioBytes: async () => new ArrayBuffer(0),
  detectCsdk: async () => "D:\\Desktop\\Reduced_CSDK_13",
  setCsdkDir: ok,
  createSoundProject: async (name, hero) => {
    const p = { id: String(Date.now()), name, hero, items: [], modId: null, lastBuilt: null };
    projects = [...projects, p];
    return p;
  },
  updateSoundProject: async (id, name, hero) => {
    projects = projects.map((p) => (p.id === id ? { ...p, name: name ?? p.name, hero: hero ?? p.hero } : p));
  },
  deleteSoundProject: async (id) => {
    projects = projects.filter((p) => p.id !== id);
  },
  setProjectItem: async (id, target, source) => {
    projects = projects.map((p) =>
      p.id !== id ? p : { ...p, items: p.items.some((i) => i.target === target) ? p.items.map((i) => (i.target === target ? { target, source } : i)) : [...p.items, { target, source }] },
    );
  },
  removeProjectItem: async (id, target) => {
    projects = projects.map((p) => (p.id === id ? { ...p, items: p.items.filter((i) => i.target !== target) } : p));
  },
  buildSoundProject: async (id) => {
    await new Promise((r) => setTimeout(r, 600));
    const p = projects.find((x) => x.id === id)!;
    return {
      modId: "a1",
      modName: p.name,
      report: {
        vpk: "x.vpk",
        vpkSize: 2_400_000,
        log: "OK: 1 compiled, 0 failed, 0 skipped",
        items: p.items.map((i, k) => ({
          target: i.target,
          original: { loudnessDb: -15.8, peakDb: -0.6, durationSecs: 18, channels: 2, rate: 44100 },
          yours: { loudnessDb: k === 1 ? -9.1 : -16.4, peakDb: -0.2, durationSecs: 18, channels: k === 1 ? 1 : 2, rate: 48000 },
          gainDb: k === 1 ? -6.7 : 0,
          looped: i.target.includes("_lp"),
          convertedChannels: k === 1,
        })),
      },
    };
  },
};
