// In-memory stand-in for the Tauri backend, used only when the UI runs in a
// plain browser (`npm run dev`) so the design can be worked on without the
// desktop app. Nothing here touches real files.

import type { Action, IconProject, ModEntry, ModelProject, ModelSummary, Overview, Pack, Profile, SoundProject } from "./types";
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
      { target: "sounds/abilities/gigawatt/gigawatt_storm_cloud_lp.vsnd_c", source: "D:\\Audio\\storm_loop_v3.wav", volumeDb: 0 },
      { target: "sounds/abilities/gigawatt/gigawatt_storm_cloud_cast.vsnd_c", source: "D:\\Audio\\thunder_cast.mp4", volumeDb: 2 },
      { target: "sounds/abilities/gigawatt/gigawatt_storm_cloud_bolt_cast.vsnd_c", source: "", volumeDb: 0 },
    ],
    muteVoice: [{ hero: "gigawatt", slot: 4 }],
    modId: "a1",
    lastBuilt: "2026-11-08T15:30:00+00:00",
  },
];
const gameSounds = [
  "a4_storm_cloud/end", "a4_storm_cloud/impact_1", "a4_storm_cloud/impact_2", "a4_storm_cloud/impact_3", "a4_storm_cloud/impact_4",
  "gigawatt_storm_cloud_aoe_lp", "gigawatt_storm_cloud_bolt_cast", "gigawatt_storm_cloud_bolt_explo_01", "gigawatt_storm_cloud_cast",
  "gigawatt_storm_cloud_end", "gigawatt_storm_cloud_lp", "gigawatt_power_surge_activate_lp", "gigawatt_static_charge_cast",
].map((s) => `sounds/abilities/gigawatt/${s}.vsnd_c`);
const heroes = [
  { code: "gigawatt", name: "Seven", abilities: [
    { slot: 1, key: "citadel_ability_lightning_ball", name: "Lightning Ball", voiceLines: 10 },
    { slot: 2, key: "citadel_ability_static_charge", name: "Static Charge", voiceLines: 10 },
    { slot: 3, key: "ability_power_surge", name: "Power Surge", voiceLines: 10 },
    { slot: 4, key: "citadel_ability_storm_cloud", name: "Storm Cloud", voiceLines: 15 },
  ] },
  { code: "inferno", name: "Infernus", abilities: [] },
];
function mockUses(p: string) {
  const f = p.split("/").pop()!.replace(".vsnd_c", "");
  const part = f.includes("storm_cloud") ? f.replace(/^.*storm_cloud_?/, "").replace(/_/g, " ") || "cast" : f;
  const slot = f.includes("storm_cloud") || p.includes("a4_") ? 4 : f.includes("lightning_ball") ? 1 : f.includes("static_charge") ? 2 : f.includes("power_surge") ? 3 : null;
  const names: Record<number, string> = { 1: "Lightning Ball", 2: "Static Charge", 3: "Power Surge", 4: "Storm Cloud" };
  const label = part.replace(/\blp\b/, "loop").replace(/explo/, "explosion");
  return [{ hero: "gigawatt", heroName: "Seven", category: slot ? "Ability" : "Other", slot, abilityName: slot ? names[slot] : null, part: label[0].toUpperCase() + label.slice(1), event: "Gigawatt.Mock" }];
}
let log: string[] = [`${now().slice(0, 19)}+00:00  Imported "Seven Storm Cloud remix" (5 files inside)`];

function plan() {
  const want = mods.filter(active).map((m) => m.id);
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
  for (const m of mods.filter(active)) for (const f of m.files) map.set(f, [...(map.get(f) ?? []), m.id]);
  return [...map].filter(([, v]) => v.length > 1).map(([path, ids]) => ({ path, mods: ids }));
}

const ok = async () => undefined;

let modelProjects: ModelProject[] = [
  { id: "m1seven", name: "Seven test rebuild", hero: "gigawatt", model: "models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner.vmdl", created: now(), extracted: null, extractedBuild: null, extractedTool: null, modId: null, lastBuilt: null },
];
let s2vInstalled = false;
let blenderExe: string | null = "C:\\Program Files\\Blender Foundation\\Blender 5.0\\blender.exe";
let sentBack = false;
const sevenSummary: ModelSummary = {
  bones: Array.from({ length: 245 }, (_, i) => (i === 0 ? "root_motion" : `bone_${i}`)),
  meshes: 20,
  lods: 4,
  bodyGroups: ["DEFAULT: BASE", "DEFAULT: NO_GUN", "DEFAULT: NO_GUN_AND_SKELETON"],
  materials: ["materials/blue_illum.vmat", "models/heroes_staging/gigawatt/materials/gig_greybox.vmat", "models/heroes_staging/gigawatt_prisoner/materials/gigawatt_prisoner.vmat", "models/heroes_staging/gigawatt_prisoner/materials/gigawatt_prisoner_gun_d.vmat", "models/heroes_staging/gigawatt_prisoner/materials/gigawatt_prisoner_head.vmat"],
  attachments: Array.from({ length: 76 }, (_, i) => `attach_${i}`),
  hitboxes: 20,
  ragdollBodies: 18,
  clothNodes: 131,
  animGraphs: ["animgraphs/animgraph2/hero/hero.vnmgraph+gigawatt.vnmgraph"],
  trianglesLod0: 269945,
};
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

let packs: Pack[] = [];
let iconProjects: IconProject[] = [];
// Same rule as the app: a pack's mods sit together at its first mod's place.
const groupPacks = () => {
  packs = packs.map((p) => ({ ...p, mods: mods.filter((m) => p.mods.includes(m.id)).map((m) => m.id) })).filter((p) => p.mods.length);
  const out: typeof mods = [];
  for (const m of mods) {
    if (out.includes(m)) continue;
    const p = packs.find((x) => x.mods.includes(m.id));
    if (p) out.push(...p.mods.map((id) => mods.find((x) => x.id === id)!));
    else out.push(m);
  }
  mods = out;
};
const active = (m: { id: string; enabled: boolean }) => m.enabled && (packs.find((p) => p.mods.includes(m.id))?.enabled ?? true);

export const mockApi: Api = {
  overview: async (): Promise<Overview> => ({
    mods: structuredClone(mods),
    packs: structuredClone(packs),
    profiles: structuredClone(profiles),
    settings: { gameDir: "D:\\Games\\SteamLibrary\\steamapps\\common\\Deadlock", csdkDir: "D:\\Desktop\\Reduced_CSDK_13", blenderExe: blenderExe, blenderAddon: null },
    safeMode: null,
    game: { dir: "D:\\Games\\SteamLibrary\\steamapps\\common\\Deadlock", found: true, buildId: BUILD, running: false, addonsLine: true, sharedAddonsLine: true },
    plan: plan(),
    conflicts: conflicts(),
    dataDir: "C:\\Users\\you\\AppData\\Roaming\\app.deadlockmodstudio.local",
    soundProjects: structuredClone(projects),
    csdkFound: true,
    modelProjects: structuredClone(modelProjects),
    iconProjects: structuredClone(iconProjects),
    s2vInstalled,
    s2vVersion: "fcf7323",
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
    groupPacks();
  },
  createPack: async (name, modIds) => {
    packs = packs.map((p) => ({ ...p, mods: p.mods.filter((m) => !modIds.includes(m)) }));
    const p: Pack = { id: String(Date.now()), name, notes: "", enabled: true, mods: modIds, created: now() };
    packs.push(p);
    groupPacks();
    return packs.find((x) => x.id === p.id)!;
  },
  updatePack: async (id, name, notes) => {
    packs = packs.map((p) => (p.id === id ? { ...p, name: name ?? p.name, notes: notes ?? p.notes } : p));
  },
  setPackEnabled: async (id, enabled) => {
    packs = packs.map((p) => (p.id === id ? { ...p, enabled } : p));
  },
  setModPack: async (modId, packId) => {
    packs = packs.map((p) => ({ ...p, mods: p.mods.filter((m) => m !== modId).concat(p.id === packId ? [modId] : []) }));
    if (packId) {
      const p = packs.find((x) => x.id === packId)!;
      const m = mods.find((x) => x.id === modId)!;
      mods = mods.filter((x) => x !== m);
      const last = p.mods[p.mods.length - 2];
      mods.splice(last ? mods.findIndex((x) => x.id === last) + 1 : mods.length, 0, m);
    }
    groupPacks();
  },
  deletePack: async (id, withMods) => {
    const p = packs.find((x) => x.id === id)!;
    packs = packs.filter((x) => x.id !== id);
    if (withMods) mods = mods.filter((m) => !p.mods.includes(m.id));
  },
  exportPack: async () => {
    await wait(400);
  },
  importPack: async () => {
    throw new Error("Importing packs needs the desktop app");
  },
  iconHeroes: async () => [
    [
      { code: "unicorn", name: "Celeste", images: [["m_strIconHeroCard", "file://{images}/heroes/unicorn_card.psd"], ["m_strIconHeroCardCritical", "file://{images}/heroes/unicorn_card_critical.psd"], ["m_strIconHeroCardGloat", "file://{images}/heroes/unicorn_card_gloat.psd"], ["m_strTopBarVertical", "file://{images}/heroes/unicorn_vertical.psd"], ["m_strIconImageSmall", "file://{images}/heroes/unicorn_sm.psd"], ["m_strMinimapImage", "file://{images}/heroes/unicorn_mm.psd"]] },
      { code: "gigawatt", name: "Seven", images: [["m_strIconHeroCard", "file://{images}/heroes/gigawatt_card.psd"]] },
    ],
    [
      { field: "m_strIconHeroCard", label: "Card", width: 280, height: 380 },
      { field: "m_strIconHeroCardCritical", label: "Card: low health", width: 280, height: 380 },
      { field: "m_strIconHeroCardGloat", label: "Card: gloating", width: 280, height: 380 },
      { field: "m_strTopBarVertical", label: "Tall icon", width: 120, height: 200 },
      { field: "m_strIconImageSmall", label: "Small icon", width: 128, height: 128 },
      { field: "m_strMinimapImage", label: "Minimap icon", width: 50, height: 50 },
    ],
  ],
  createIconProject: async (name, hero) => {
    const p: IconProject = { id: String(Date.now()), name, hero, images: {}, modId: null, lastBuilt: null };
    iconProjects = [...iconProjects, p];
    return p;
  },
  updateIconProject: async (id, name, field, source) => {
    iconProjects = iconProjects.map((p) => {
      if (p.id !== id) return p;
      const images = { ...p.images };
      if (field) {
        if (source) images[field] = source;
        else delete images[field];
      }
      return { ...p, name: name ?? p.name, images };
    });
  },
  deleteIconProject: async (id) => {
    iconProjects = iconProjects.filter((p) => p.id !== id);
  },
  iconPreview: async (field) => {
    const slot = { m_strIconHeroCard: [280, 380], m_strIconHeroCardCritical: [280, 380], m_strIconHeroCardGloat: [280, 380], m_strTopBarVertical: [120, 200], m_strIconImageSmall: [128, 128], m_strMinimapImage: [50, 50] }[field] ?? [100, 100];
    const c = document.createElement("canvas");
    c.width = slot[0];
    c.height = slot[1];
    const g = c.getContext("2d")!;
    const grad = g.createLinearGradient(0, 0, 0, slot[1]);
    grad.addColorStop(0, "#c9c9c9");
    grad.addColorStop(1, "#3b3b3b");
    g.fillStyle = grad;
    g.fillRect(0, 0, slot[0], slot[1]);
    const blob: Blob = await new Promise((r) => c.toBlob((b) => r(b!), "image/png"));
    return blob.arrayBuffer();
  },
  buildIconProject: async (id) => {
    await wait(700);
    const p = iconProjects.find((x) => x.id === id)!;
    iconProjects = iconProjects.map((x) => (x.id === id ? { ...x, modId: "a1", lastBuilt: now() } : x));
    return { modId: "a1", modName: p.name, report: { files: Object.keys(p.images).map((f) => `panorama/images/heroes/${p.hero}_${f}_psd.vtex_c`), vpk: "x.vpk", vpkSize: 1_200_000, log: "OK" } };
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
    applied = mods.filter(active).map((m) => m.id);
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
  searchGameSounds: async (query, hero, group) => {
    const w = query.toLowerCase().split(/\s+/).filter(Boolean);
    const r = gameSounds.filter((p) => w.every((x) => (p + " " + mockUses(p).map((u) => `${u.abilityName} ${u.part}`).join(" ")).toLowerCase().includes(x)))
      .filter((p) => !hero || mockUses(p).some((u) => u.hero === hero))
      .filter((p) => !group || mockUses(p).some((u) => (group.startsWith("slot:") ? `slot:${u.slot}` === group : u.category === group)));
    return { total: r.length, results: r.map((path) => ({ path, size: 150000, looped: /_lp\./.test(path), uses: mockUses(path) })) };
  },
  soundHeroes: async () => heroes,
  describeSounds: async (paths) => Object.fromEntries(paths.map((p) => [p, mockUses(p)])),
  setItemVolume: async (id, target, volumeDb) => {
    projects = projects.map((p) => (p.id !== id ? p : { ...p, items: p.items.map((i) => (i.target === target ? { ...i, volumeDb: Math.min(3, volumeDb) } : i)) }));
  },
  setVoiceMute: async (id, hero, slot, on) => {
    projects = projects.map((p) =>
      p.id !== id ? p : { ...p, muteVoice: [...p.muteVoice.filter((m) => !(m.hero === hero && m.slot === slot)), ...(on ? [{ hero, slot }] : [])] },
    );
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
    const p = { id: String(Date.now()), name, hero, items: [], muteVoice: [], modId: null, lastBuilt: null };
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
      p.id !== id ? p : { ...p, items: p.items.some((i) => i.target === target) ? p.items.map((i) => (i.target === target ? { ...i, source } : i)) : [...p.items, { target, source, volumeDb: 0 }] },
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
        muted: 15,
        items: p.items.filter((i) => i.source).map((i, k) => ({
          target: i.target,
          original: { loudnessDb: -15.8, peakDb: -0.6, durationSecs: 18, channels: 2, rate: 44100 },
          yours: { loudnessDb: k === 1 ? -9.1 : -21.4, peakDb: -0.2, durationSecs: 18, channels: k === 1 ? 1 : 2, rate: 48000 },
          result: { loudnessDb: -15.8 + i.volumeDb, peakDb: -0.5, durationSecs: 18, channels: 2, rate: 44100 },
          gainDb: k === 1 ? -4.7 : 5.6,
          volumeDb: i.volumeDb,
          limited: k === 1,
          looped: i.target.includes("_lp"),
          convertedChannels: k === 1,
        })),
      },
    };
  },
  modelHeroes: async () => [
    { code: "gigawatt", name: "Seven", model: "models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner.vmdl" },
    { code: "haze", name: "Haze", model: "models/heroes_staging/haze/haze.vmdl" },
    { code: "bebop", name: "Bebop", model: "models/heroes_staging/bebop/bebop.vmdl" },
  ],
  createModelProject: async (name, hero) => {
    const p: ModelProject = { id: `m${Date.now()}`, name, hero, model: `models/heroes_staging/${hero}/${hero}.vmdl`, created: now(), extracted: null, extractedBuild: null, extractedTool: null, modId: null, lastBuilt: null };
    modelProjects = [...modelProjects, p];
    return p;
  },
  renameModelProject: async (id, name) => {
    modelProjects = modelProjects.map((p) => (p.id === id ? { ...p, name } : p));
  },
  deleteModelProject: async (id) => {
    modelProjects = modelProjects.filter((p) => p.id !== id);
  },
  modelSummary: async (id) => (modelProjects.find((p) => p.id === id)?.extracted ? sevenSummary : null),
  openModelFolder: ok,
  extractModelProject: async (id) => {
    await wait(900);
    s2vInstalled = true;
    modelProjects = modelProjects.map((p) => (p.id === id ? { ...p, extracted: now(), extractedBuild: BUILD, extractedTool: "fcf7323" } : p));
    return { project: modelProjects.find((p) => p.id === id)!, report: { summary: sevenSummary, blenderFile: "C:\\x\\blender\\gigawatt_prisoner.glb", template: "x.vmdl", meshFiles: [], clothMissing: false, log: "" } };
  },
  buildModelProject: async (id, withModel) => {
    await wait(900);
    const p = modelProjects.find((x) => x.id === id)!;
    modelProjects = modelProjects.map((x) => (x.id === id ? { ...x, lastBuilt: now(), modId: "a1" } : x));
    return {
      modId: "a1",
      modName: p.name,
      report: {
        parts: withModel
          ? [
              { part: "outer_costume", status: "replaced", vertices: 67961, triangles: 107910 },
              { part: "gun", status: "kept", vertices: 0, triangles: 0 },
              { part: "headgear", status: "replaced", vertices: 13434, triangles: 22515 },
            ]
          : [],
        materials: withModel ? [{ material: "gigawatt_prisoner", path: "models/heroes_staging/gigawatt_prisoner/materials/gigawatt_prisoner.vmat", color: "x_dms_color.png", size: [2048, 2048], resized: false, new: false }, { material: "Top Hat Felt", path: "models/heroes_staging/gigawatt_prisoner/dms_materials/top_hat_felt.vmat", color: "top_hat_felt_color.png", size: [4, 4], resized: false, new: true }] : [],
        vpk: "x.vpk", vpkSize: 21_400_000, files: [`${p.model}_c`, p.model.replace(".vmdl", "_backup.vmdl_c")], warnings: ["WARNING: Unknown game data class CCitadelHeroModelGameData_t", ...Array.from({ length: 7 }, (_, i) => `RESOURCE COMPILE WARNING: ClothChain: arm_lower_R : 2 warnings, while processing node ${i} of models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner.vmdl`)], log: "OK: 1 compiled, 0 failed, 0 skipped" },
    };
  },
  detectBlender: async () => [{ exe: "C:\\Program Files\\Blender Foundation\\Blender 5.0\\blender.exe", version: "5.0" }],
  setBlenderExe: async (path) => {
    blenderExe = path;
  },
  editInBlender: async () => {
    await wait(500);
    sentBack = true;
    return "Blender is opening and importing the hero. It saves a work file in the project's blender folder.";
  },
  modelReturns: async () =>
    sentBack
      ? [{ folder: "x", sent: now(), blender: "5.0.0", addon: "0.1.0", glb: "model.glb", objects: [{ object: "outer_costume", part: "outer_costume", triangles: 107910 }, { object: "gun", part: "gun", triangles: 32803 }], materials: ["gigawatt_prisoner"], materialFiles: [{ name: "gigawatt_prisoner", color: "textures/gigawatt_prisoner_color.png" }] }]
      : [],
  // Dev only: put model.glb, original.glb and textures in public/dev-preview/
  // (not committed) to try the preview in a browser.
  modelPreviewInfo: async (_id, original = false) => ({
    model: original || !sentBack ? "original.glb" : "model.glb",
    sent: original || !sentBack ? null : now(),
    original: "original.glb",
    materials: {
      gigawatt_prisoner: { outline: [0.063, 0.02, 0.02], glow: { tint: [0.75, 0.38, 0.04], scale: 10, mask: "gigawatt_prisoner_emissive.png" }, color: "gigawatt_prisoner_color_png_288151fd.png", ao: "gigawatt_prisoner_ao.png", metal: "gigawatt_prisoner_288151fd_metal.png", roughness: 0.97, isNew: false },
      Top_Hat_Test: { outline: [0.063, 0.02, 0.02], glow: null, color: null, ao: null, metal: null, roughness: 1, isNew: true },
    },
  }),
  modelPreviewFile: async (_id, path) => {
    const r = await fetch(`/dev-preview/${path.split(/[\\/]/).pop()}`);
    if (!r.ok) throw new Error(`No ${path} in public/dev-preview`);
    return r.arrayBuffer();
  },
};
