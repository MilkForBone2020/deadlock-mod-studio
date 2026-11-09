import { invoke } from "@tauri-apps/api/core";
import type { ApplyReport, BlenderInstall, Returned, BuildResult, ExtractResult, HeroInfo, Level, ModelBuildResult, ModelHero, ModelProject, ModelSummary, ModMeta, Pack, PreviewInfo, IconBuildResult, IconHero, IconProject, IconSlot, Overview, Profile, SafeModeEvent, SoundProject, SoundSearch, SoundUse } from "./types";
import { mockApi } from "./mock";

/** False when the UI is opened in a plain browser (`npm run dev`) for design work. */
export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// One typed wrapper per Tauri command in src-tauri/src/lib.rs.
const tauriApi = {
  overview: () => invoke<Overview>("get_overview"),
  checkForUpdate: () => invoke<SafeModeEvent | null>("check_for_update"),
  dismissSafeMode: () => invoke<void>("dismiss_safe_mode"),
  detectGameDir: () => invoke<string | null>("detect_game_dir"),
  setGameDir: (path: string) => invoke<void>("set_game_dir", { path }),
  importMods: (paths: string[]) => invoke<string[]>("import_mods", { paths }),
  replaceModFile: (id: string, path: string) => invoke<void>("replace_mod_file", { id, path }),
  updateMod: (id: string, meta: ModMeta) => invoke<void>("update_mod", { id, meta }),
  removeMod: (id: string) => invoke<void>("remove_mod", { id }),
  setEnabled: (id: string, enabled: boolean) => invoke<void>("set_enabled", { id, enabled }),
  setAllEnabled: (enabled: boolean) => invoke<void>("set_all_enabled", { enabled }),
  reorder: (ids: string[]) => invoke<void>("reorder", { ids }),
  createPack: (name: string, modIds: string[]) => invoke<Pack>("create_pack", { name, modIds }),
  updatePack: (id: string, name?: string, notes?: string) => invoke<void>("update_pack", { id, name, notes }),
  setPackEnabled: (id: string, enabled: boolean) => invoke<void>("set_pack_enabled", { id, enabled }),
  setModPack: (modId: string, packId: string | null) => invoke<void>("set_mod_pack", { modId, packId }),
  deletePack: (id: string, withMods: boolean) => invoke<void>("delete_pack", { id, withMods }),
  exportPack: (id: string, path: string) => invoke<void>("export_pack", { id, path }),
  importPack: (path: string) => invoke<Pack>("import_pack", { path }),
  iconHeroes: () => invoke<[IconHero[], IconSlot[]]>("icon_heroes"),
  createIconProject: (name: string, hero: string) => invoke<IconProject>("create_icon_project", { name, hero }),
  updateIconProject: (id: string, name?: string, field?: string, source?: string | null) => invoke<void>("update_icon_project", { id, name, field, source }),
  deleteIconProject: (id: string) => invoke<void>("delete_icon_project", { id }),
  iconPreview: (field: string, source: string) => invoke<ArrayBuffer>("icon_preview", { field, source }),
  buildIconProject: (id: string) => invoke<IconBuildResult>("build_icon_project", { id }),
  markVerified: (id: string, verified: boolean) => invoke<void>("mark_verified", { id, verified }),
  saveProfile: (name: string) => invoke<Profile>("save_profile", { name }),
  deleteProfile: (id: string) => invoke<void>("delete_profile", { id }),
  applyProfile: (id: string) => invoke<void>("apply_profile", { id }),
  applyChanges: () => invoke<ApplyReport>("apply_changes"),
  restoreVanilla: () => invoke<ApplyReport>("restore_vanilla"),
  readLog: (lines: number) => invoke<string[]>("read_log", { lines }),
  openFolder: (which: "library" | "backups" | "logs" | "addons" | "game") =>
    invoke<void>("open_folder", { which }),
  launchGame: () => invoke<void>("launch_game"),
  searchGameSounds: (query: string, hero: string | null, group: string | null, limit = 300) =>
    invoke<SoundSearch>("search_game_sounds", { query, hero, group, limit }),
  soundHeroes: () => invoke<HeroInfo[]>("sound_heroes"),
  describeSounds: (paths: string[]) => invoke<Record<string, SoundUse[]>>("describe_sounds", { paths }),
  setItemVolume: (id: string, target: string, volumeDb: number) => invoke<void>("set_item_volume", { id, target, volumeDb }),
  setVoiceMute: (id: string, hero: string, slot: number, on: boolean) => invoke<void>("set_voice_mute", { id, hero, slot, on }),
  audioLevel: (kind: "game" | "file", path: string) => invoke<Level>("audio_level", { kind, path }),
  audioBytes: (kind: "game" | "file", path: string) => invoke<ArrayBuffer>("audio_bytes", { kind, path }),
  detectCsdk: () => invoke<string | null>("detect_csdk"),
  setCsdkDir: (path: string) => invoke<void>("set_csdk_dir", { path }),
  createSoundProject: (name: string, hero: string) => invoke<SoundProject>("create_sound_project", { name, hero }),
  updateSoundProject: (id: string, name?: string, hero?: string) => invoke<void>("update_sound_project", { id, name, hero }),
  deleteSoundProject: (id: string) => invoke<void>("delete_sound_project", { id }),
  setProjectItem: (id: string, target: string, source: string) => invoke<void>("set_project_item", { id, target, source }),
  removeProjectItem: (id: string, target: string) => invoke<void>("remove_project_item", { id, target }),
  buildSoundProject: (id: string) => invoke<BuildResult>("build_sound_project", { id }),
  modelHeroes: () => invoke<ModelHero[]>("model_heroes"),
  createModelProject: (name: string, hero: string) => invoke<ModelProject>("create_model_project", { name, hero }),
  renameModelProject: (id: string, name: string) => invoke<void>("rename_model_project", { id, name }),
  deleteModelProject: (id: string) => invoke<void>("delete_model_project", { id }),
  modelSummary: (id: string) => invoke<ModelSummary | null>("model_summary", { id }),
  openModelFolder: (id: string, which: "blender" | "return" | "original" | "project") => invoke<void>("open_model_folder", { id, which }),
  extractModelProject: (id: string) => invoke<ExtractResult>("extract_model_project", { id }),
  buildModelProject: (id: string, withModel: boolean) => invoke<ModelBuildResult>("build_model_project", { id, withModel }),
  detectBlender: () => invoke<BlenderInstall[]>("detect_blender"),
  setBlenderExe: (path: string) => invoke<void>("set_blender_exe", { path }),
  editInBlender: (id: string, fresh = false) => invoke<string>("edit_in_blender", { id, fresh }),
  modelReturns: (id: string) => invoke<Returned[]>("model_returns", { id }),
  modelPreviewInfo: (id: string, original = false) => invoke<PreviewInfo>("model_preview_info", { id, original }),
  modelPreviewFile: (id: string, path: string) => invoke<ArrayBuffer>("model_preview_file", { id, path }),
};

export type Api = typeof tauriApi;

export const api: Api = isTauri ? tauriApi : mockApi;

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
