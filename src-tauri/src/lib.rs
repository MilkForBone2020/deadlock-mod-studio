//! Tauri commands. All file logic lives in `modcore`; this layer adds the
//! game-running guard, picks the data folder and talks to the UI.

use modcore::audio::{self, Level};
use modcore::catalog::{self, Catalog, HeroInfo, SoundUse};
use modcore::csdk::{self, BuildReport};
use modcore::library::{BuildCheck, Library, ModEntry, ModMeta, ModelProject, Profile, SafeModeEvent, Settings, SoundProject};
use modcore::model::{self, ExtractReport, ModelBuildReport, ModelSummary};
use modcore::tools;
use modcore::vpk::Directory;
use modcore::sync::{self, ApplyReport, Plan};
use modcore::{steam, Error};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;
use tauri::{Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

struct App {
    lib: Mutex<Library>,
    /// Deadlock's pak01 directory, cached until the file changes.
    game_index: Mutex<Option<(PathBuf, SystemTime, Arc<Directory>)>>,
    /// Sound catalogue (who/which ability each sound belongs to), cached
    /// per pak01 version.
    catalog: Mutex<Option<(SystemTime, Arc<Catalog>)>>,
}

type CmdResult<T> = Result<T, String>;

fn err(e: Error) -> String {
    e.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GameStatus {
    dir: Option<String>,
    found: bool,
    build_id: Option<String>,
    running: bool,
    /// Whether gameinfo.gi currently loads this app's addons_studio folder.
    addons_line: Option<bool>,
    /// Whether another manager's `Game citadel/addons` line is present.
    shared_addons_line: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Overview {
    mods: Vec<ModEntry>,
    profiles: Vec<Profile>,
    settings: Settings,
    safe_mode: Option<SafeModeEvent>,
    game: GameStatus,
    plan: Plan,
    conflicts: Vec<modcore::library::Conflict>,
    data_dir: String,
    sound_projects: Vec<SoundProject>,
    csdk_found: bool,
    model_projects: Vec<ModelProject>,
    /// Whether Source 2 Viewer's command-line tool is installed.
    s2v_installed: bool,
}

/// Is Deadlock (project8.exe) running right now?
fn game_running() -> bool {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    sys.processes().values().any(|p| {
        let n = p.name().to_string_lossy().to_ascii_lowercase();
        n == "project8.exe" || n == "project8"
    })
}

fn game_dir(lib: &Library) -> Option<PathBuf> {
    lib.state.settings.game_dir.as_ref().map(PathBuf::from)
}

fn require_game_dir(lib: &Library) -> CmdResult<PathBuf> {
    game_dir(lib).ok_or_else(|| "Set the Deadlock folder in Settings first".to_string())
}

fn ensure_closed() -> CmdResult<()> {
    if game_running() {
        Err("Deadlock is running. Close the game first, then apply changes.".into())
    } else {
        Ok(())
    }
}

fn overview_of(lib: &Library) -> CmdResult<Overview> {
    let dir = game_dir(lib);
    let found = dir.as_deref().map(steam::is_game_dir).unwrap_or(false);
    let build_id = dir.as_deref().and_then(steam::build_id);
    let gi_text = dir
        .as_deref()
        .filter(|_| found)
        .and_then(|d| std::fs::read_to_string(steam::gameinfo_path(d)).ok());
    let addons_line = gi_text.as_deref().and_then(|t| modcore::gameinfo::has_addons_path(t).ok());
    let shared_addons_line =
        gi_text.as_deref().and_then(|t| modcore::gameinfo::has_shared_addons_path(t).ok());
    let plan = match dir.as_deref() {
        Some(d) => sync::plan(lib, d).map_err(err)?,
        None => Plan {
            blockers: vec!["Set the Deadlock folder in Settings first".into()],
            ..Plan::default()
        },
    };
    Ok(Overview {
        mods: lib.state.mods.clone(),
        profiles: lib.state.profiles.clone(),
        settings: lib.state.settings.clone(),
        safe_mode: lib.state.safe_mode.clone(),
        game: GameStatus {
            dir: dir.map(|d| d.display().to_string()),
            found,
            build_id,
            running: game_running(),
            addons_line,
            shared_addons_line,
        },
        plan,
        conflicts: lib.conflicts(),
        data_dir: lib.root.display().to_string(),
        sound_projects: lib.state.sound_projects.clone(),
        csdk_found: lib.state.settings.csdk_dir.as_deref().map(|d| csdk::is_csdk_dir(Path::new(d))).unwrap_or(false),
        model_projects: lib.state.model_projects.clone(),
        s2v_installed: tools::s2v_exe(&lib.root.join("tools")).is_some(),
    })
}

#[tauri::command]
fn get_overview(app: State<App>) -> CmdResult<Overview> {
    let lib = app.lib.lock().unwrap();
    overview_of(&lib)
}

/// Run at startup and on a timer. On a new Deadlock build every mod is
/// switched off, and the addons_studio folder is cleared straight away if the
/// game isn't running (otherwise it waits as a pending change).
#[tauri::command]
fn check_for_update(app: State<App>) -> CmdResult<Option<SafeModeEvent>> {
    let mut lib = app.lib.lock().unwrap();
    let Some(dir) = game_dir(&lib) else { return Ok(None) };
    let build = steam::build_id(&dir);
    match lib.check_build(build.as_deref()).map_err(err)? {
        BuildCheck::Changed(ev) => {
            if !game_running() {
                if let Err(e) = sync::apply(&mut lib, &dir) {
                    lib.log(&format!("Safe mode could not clear addons_studio yet: {e}"));
                }
            }
            Ok(Some(ev))
        }
        _ => Ok(None),
    }
}

#[tauri::command]
fn dismiss_safe_mode(app: State<App>) -> CmdResult<()> {
    app.lib.lock().unwrap().dismiss_safe_mode().map_err(err)
}

#[tauri::command]
fn detect_game_dir() -> Option<String> {
    steam::detect_game_dir().map(|p| p.display().to_string())
}

#[tauri::command]
fn set_game_dir(app: State<App>, path: String) -> CmdResult<()> {
    let p = PathBuf::from(path.trim());
    if !steam::is_game_dir(&p) {
        return Err(format!(
            "{} doesn't look like Deadlock: game\\citadel\\gameinfo.gi wasn't found",
            p.display()
        ));
    }
    let mut lib = app.lib.lock().unwrap();
    lib.state.settings.game_dir = Some(p.display().to_string());
    lib.log(&format!("Deadlock folder set to {}", p.display()));
    lib.save().map_err(err)
}

#[tauri::command]
fn import_mods(app: State<App>, paths: Vec<String>) -> CmdResult<Vec<String>> {
    let mut lib = app.lib.lock().unwrap();
    let mut problems = Vec::new();
    for p in paths {
        if let Err(e) = lib.import_vpk(Path::new(&p), ModMeta::default()) {
            problems.push(e.to_string());
        }
    }
    Ok(problems)
}

#[tauri::command]
fn replace_mod_file(app: State<App>, id: String, path: String) -> CmdResult<()> {
    app.lib.lock().unwrap().replace_vpk(&id, Path::new(&path)).map_err(err)
}

#[tauri::command]
fn update_mod(app: State<App>, id: String, meta: ModMeta) -> CmdResult<()> {
    app.lib.lock().unwrap().update_meta(&id, meta).map_err(err)
}

#[tauri::command]
fn remove_mod(app: State<App>, id: String) -> CmdResult<()> {
    app.lib.lock().unwrap().remove_mod(&id).map_err(err)
}

#[tauri::command]
fn set_enabled(app: State<App>, id: String, enabled: bool) -> CmdResult<()> {
    app.lib.lock().unwrap().set_enabled(&id, enabled).map_err(err)
}

#[tauri::command]
fn set_all_enabled(app: State<App>, enabled: bool) -> CmdResult<()> {
    app.lib.lock().unwrap().set_all_enabled(enabled).map_err(err)
}

#[tauri::command]
fn reorder(app: State<App>, ids: Vec<String>) -> CmdResult<()> {
    app.lib.lock().unwrap().reorder(&ids).map_err(err)
}

#[tauri::command]
fn mark_verified(app: State<App>, id: String, verified: bool) -> CmdResult<()> {
    let mut lib = app.lib.lock().unwrap();
    let build = if verified {
        let dir = require_game_dir(&lib)?;
        Some(steam::build_id(&dir).ok_or("Couldn't read Deadlock's build ID from Steam")?)
    } else {
        None
    };
    lib.mark_verified(&id, build).map_err(err)
}

#[tauri::command]
fn save_profile(app: State<App>, name: String) -> CmdResult<Profile> {
    app.lib.lock().unwrap().save_profile(&name).map_err(err)
}

#[tauri::command]
fn delete_profile(app: State<App>, id: String) -> CmdResult<()> {
    app.lib.lock().unwrap().delete_profile(&id).map_err(err)
}

#[tauri::command]
fn apply_profile(app: State<App>, id: String) -> CmdResult<()> {
    app.lib.lock().unwrap().apply_profile(&id).map_err(err)
}

#[tauri::command]
fn apply_changes(app: State<App>) -> CmdResult<ApplyReport> {
    ensure_closed()?;
    let mut lib = app.lib.lock().unwrap();
    let dir = require_game_dir(&lib)?;
    sync::apply(&mut lib, &dir).map_err(err)
}

/// Switch every mod off and make the game folder vanilla again: managed
/// VPKs removed, the addons_studio line taken out of gameinfo.gi.
#[tauri::command]
fn restore_vanilla(app: State<App>) -> CmdResult<ApplyReport> {
    ensure_closed()?;
    let mut lib = app.lib.lock().unwrap();
    let dir = require_game_dir(&lib)?;
    lib.set_all_enabled(false).map_err(err)?;
    lib.log("Restore vanilla requested");
    sync::apply(&mut lib, &dir).map_err(err)
}

#[tauri::command]
fn read_log(app: State<App>, lines: usize) -> Vec<String> {
    let lib = app.lib.lock().unwrap();
    let text = std::fs::read_to_string(lib.log_path()).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].iter().rev().map(|s| s.to_string()).collect()
}

#[tauri::command]
fn open_folder(app_handle: tauri::AppHandle, app: State<App>, which: String) -> CmdResult<()> {
    let lib = app.lib.lock().unwrap();
    let path = match which.as_str() {
        "library" => lib.library_dir(),
        "backups" => lib.backups_dir(),
        "logs" => lib.root.join("logs"),
        "addons" => steam::addons_dir(&require_game_dir(&lib)?),
        "game" => steam::citadel_dir(&require_game_dir(&lib)?),
        _ => return Err("Unknown folder".into()),
    };
    if !path.exists() {
        return Err(format!("{} doesn't exist yet", path.display()));
    }
    app_handle
        .opener()
        .open_path(path.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn launch_game(app_handle: tauri::AppHandle) -> CmdResult<()> {
    app_handle
        .opener()
        .open_url(format!("steam://rungameid/{}", steam::APP_ID), None::<&str>)
        .map_err(|e| e.to_string())
}

// ---------- Game sounds ----------

fn steam_running() -> bool {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    sys.processes().values().any(|p| {
        let n = p.name().to_string_lossy().to_ascii_lowercase();
        n == "steam.exe" || n == "steam"
    })
}

fn game_index(app: &App) -> CmdResult<Arc<Directory>> {
    let dir = {
        let lib = app.lib.lock().unwrap();
        require_game_dir(&lib)?
    };
    let pak = steam::citadel_dir(&dir).join("pak01_dir.vpk");
    let mtime = std::fs::metadata(&pak)
        .and_then(|m| m.modified())
        .map_err(|_| format!("Can't read {}", pak.display()))?;
    let mut cache = app.game_index.lock().unwrap();
    if let Some((p, t, d)) = cache.as_ref() {
        if *p == pak && *t == mtime {
            return Ok(d.clone());
        }
    }
    let d = Arc::new(modcore::vpk::read_directory(&pak).map_err(err)?);
    *cache = Some((pak, mtime, d.clone()));
    Ok(d)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GameSound {
    path: String,
    size: u64,
    looped: bool,
    uses: Vec<SoundUse>,
}

fn pak_mtime(app: &App) -> CmdResult<SystemTime> {
    let dir = {
        let lib = app.lib.lock().unwrap();
        require_game_dir(&lib)?
    };
    std::fs::metadata(steam::citadel_dir(&dir).join("pak01_dir.vpk"))
        .and_then(|m| m.modified())
        .map_err(|e| e.to_string())
}

/// Build (once per game version) and return the sound catalogue.
async fn catalog_of(app: &App) -> CmdResult<Arc<Catalog>> {
    let mtime = pak_mtime(app)?;
    if let Some((t, c)) = app.catalog.lock().unwrap().as_ref() {
        if *t == mtime {
            return Ok(c.clone());
        }
    }
    let d = game_index(app)?;
    let loc = {
        let lib = app.lib.lock().unwrap();
        require_game_dir(&lib)?.join("game").join("citadel").join("resource").join("localization")
    };
    let c = tauri::async_runtime::spawn_blocking(move || Arc::new(Catalog::build(&d, Some(&loc))))
        .await
        .map_err(|e| e.to_string())?;
    *app.catalog.lock().unwrap() = Some((mtime, c.clone()));
    Ok(c)
}

/// Heroes with their abilities (by slot) and voice-line counts.
#[tauri::command]
async fn sound_heroes(app: State<'_, App>) -> CmdResult<Vec<HeroInfo>> {
    Ok(catalog_of(&app).await?.heroes.clone())
}

/// Where each of these game sounds is used.
#[tauri::command]
async fn describe_sounds(app: State<'_, App>, paths: Vec<String>) -> CmdResult<std::collections::HashMap<String, Vec<SoundUse>>> {
    let c = catalog_of(&app).await?;
    Ok(paths.into_iter().map(|p| { let u = c.uses_of(&p).to_vec(); (p, u) }).collect())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SoundSearch {
    total: usize,
    results: Vec<GameSound>,
}

/// Every word in `query` must appear in the path or in what the sound is
/// used for (hero, ability, part, event). Optional filters: a hero code and
/// a group (`slot:1`..`slot:4`, or a category such as `Weapon`).
#[tauri::command]
async fn search_game_sounds(
    app: State<'_, App>,
    query: String,
    hero: Option<String>,
    group: Option<String>,
    limit: usize,
) -> CmdResult<SoundSearch> {
    let d = game_index(&app)?;
    let cat = catalog_of(&app).await?;
    let words: Vec<String> = query
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || c == '/' || c == '\\')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect();
    let mut total = 0;
    let mut results = Vec::new();
    let hero = hero.filter(|h| !h.is_empty());
    let group = group.filter(|g| !g.is_empty());
    for e in d.entries.iter().filter(|e| e.path.starts_with("sounds/") && e.path.ends_with(".vsnd_c")) {
        let uses = cat.uses_of(&e.path);
        if let Some(h) = &hero {
            if !uses.iter().any(|u| &u.hero == h) {
                continue;
            }
        }
        if let Some(g) = &group {
            let ok = uses.iter().any(|u| {
                hero.as_ref().is_none_or(|h| &u.hero == h)
                    && match g.strip_prefix("slot:") {
                        Some(n) => u.slot.map(|s| s.to_string()) == Some(n.to_string()) && u.category != "Voice",
                        None => &u.category == g,
                    }
            });
            if !ok {
                continue;
            }
        }
        let haystack = || {
            let mut h = e.path.clone();
            for u in uses {
                h.push(' ');
                h.push_str(&format!("{} {} {} {} {}", u.hero_name, u.ability_name.clone().unwrap_or_default(), u.part, u.event, u.category).to_lowercase());
            }
            h
        };
        let hs = if words.is_empty() { String::new() } else { haystack() };
        if words.iter().all(|w| hs.contains(w.as_str())) {
            total += 1;
            if results.len() < limit.min(500) {
                results.push(GameSound { path: e.path.clone(), size: e.size(), looped: csdk::is_loop(&e.path), uses: uses.to_vec() });
            }
        }
    }
    Ok(SoundSearch { total, results })
}

fn game_sound_bytes(app: &App, path: &str) -> CmdResult<Vec<u8>> {
    let d = game_index(app)?;
    let e = d.find(path).ok_or_else(|| format!("{path} isn't in the game's files"))?;
    d.extract(e).map_err(err)
}

/// Loudness, peak, length and format of a game sound or one of my files.
#[tauri::command]
async fn audio_level(app: State<'_, App>, kind: String, path: String) -> CmdResult<Level> {
    let pcm = if kind == "game" {
        let bytes = game_sound_bytes(&app, &path)?;
        tauri::async_runtime::spawn_blocking(move || audio::decode_vsnd(&bytes))
            .await
            .map_err(|e| e.to_string())?
    } else {
        let p = PathBuf::from(&path);
        tauri::async_runtime::spawn_blocking(move || audio::decode_file(&p))
            .await
            .map_err(|e| e.to_string())?
    }
    .map_err(err)?;
    Ok(audio::measure(&pcm))
}

/// Raw audio bytes for playback in the app (the original inside a game
/// sound, or one of my files as-is).
#[tauri::command]
async fn audio_bytes(app: State<'_, App>, kind: String, path: String) -> CmdResult<tauri::ipc::Response> {
    let bytes = if kind == "game" {
        let v = game_sound_bytes(&app, &path)?;
        audio::vsnd_audio(&v).map_err(err)?.to_vec()
    } else {
        std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?
    };
    Ok(tauri::ipc::Response::new(bytes))
}

// ---------- CSDK + sound projects ----------

#[tauri::command]
fn detect_csdk() -> Option<String> {
    csdk::detect().map(|p| p.display().to_string())
}

#[tauri::command]
fn set_csdk_dir(app: State<App>, path: String) -> CmdResult<()> {
    let p = PathBuf::from(path.trim());
    if !csdk::is_csdk_dir(&p) {
        return Err(format!(
            "{} doesn't look like CSDK: game\\bin\\win64\\resourcecompiler.exe wasn't found",
            p.display()
        ));
    }
    let mut lib = app.lib.lock().unwrap();
    lib.state.settings.csdk_dir = Some(p.display().to_string());
    lib.log(&format!("CSDK folder set to {}", p.display()));
    lib.save().map_err(err)
}

#[tauri::command]
fn create_sound_project(app: State<App>, name: String, hero: String) -> CmdResult<SoundProject> {
    app.lib.lock().unwrap().create_project(&name, &hero).map_err(err)
}

#[tauri::command]
fn update_sound_project(app: State<App>, id: String, name: Option<String>, hero: Option<String>) -> CmdResult<()> {
    app.lib.lock().unwrap().update_project(&id, name, hero).map_err(err)
}

#[tauri::command]
fn delete_sound_project(app: State<App>, id: String) -> CmdResult<()> {
    app.lib.lock().unwrap().delete_project(&id).map_err(err)
}

#[tauri::command]
fn set_project_item(app: State<App>, id: String, target: String, source: String) -> CmdResult<()> {
    app.lib.lock().unwrap().set_project_item(&id, &target, &source).map_err(err)
}

#[tauri::command]
fn set_item_volume(app: State<App>, id: String, target: String, volume_db: f64) -> CmdResult<()> {
    app.lib.lock().unwrap().set_item_volume(&id, &target, volume_db).map_err(err)
}

#[tauri::command]
fn set_voice_mute(app: State<App>, id: String, hero: String, slot: u8, on: bool) -> CmdResult<()> {
    app.lib.lock().unwrap().set_voice_mute(&id, &hero, slot, on).map_err(err)
}

#[tauri::command]
fn remove_project_item(app: State<App>, id: String, target: String) -> CmdResult<()> {
    app.lib.lock().unwrap().remove_project_item(&id, &target).map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BuildResult {
    report: BuildReport,
    mod_id: String,
    mod_name: String,
}

/// Convert, volume-match, compile and pack a sound project, then put the
/// result in the library. Progress goes out as `build-progress` events.
#[tauri::command]
async fn build_sound_project(app_handle: tauri::AppHandle, app: State<'_, App>, id: String) -> CmdResult<BuildResult> {
    let (project, csdk_dir, game_dir, out) = {
        let lib = app.lib.lock().unwrap();
        let p = lib.project(&id).map_err(err)?.clone();
        let c = lib
            .state
            .settings
            .csdk_dir
            .clone()
            .ok_or("Set the CSDK folder in Settings first")?;
        let g = require_game_dir(&lib)?;
        let safe: String = p
            .name
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
            .collect();
        let out = lib.root.join("builds").join(format!("{}_{}.vpk", safe.trim_matches('_'), &p.id[..4]));
        (p, PathBuf::from(c), g, out)
    };
    if !steam_running() {
        return Err("Start Steam first: CSDK's compiler needs it running.".into());
    }
    let slug = format!("{}_{}", project.name, &project.id[..4]);
    let jobs = project.items.clone();
    let mutes: Vec<String> = {
        let d = game_index(&app)?;
        project.mute_voice.iter().flat_map(|m| catalog::voice_lines(&d, &m.hero, m.slot)).collect()
    };
    let h = app_handle.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        csdk::build_sound_mod(&csdk_dir, &game_dir, &slug, &jobs, &mutes, &out, &csdk::run_compiler, &|m| {
            let _ = h.emit("build-progress", m.to_string());
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(err)?;

    let mut lib = app.lib.lock().unwrap();
    let mod_id = lib.store_build(&id, Path::new(&report.vpk)).map_err(err)?;
    let mod_name = lib.get(&mod_id).map(|m| m.name.clone()).unwrap_or_default();
    lib.log(&format!(
        "Built sound mod \"{}\": {} sound(s), {} voice line(s) muted",
        mod_name,
        report.items.len(),
        report.muted
    ));
    Ok(BuildResult { report, mod_id, mod_name })
}

// ---------- Model Builder ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelHero {
    code: String,
    name: String,
    model: String,
}

fn hero_model_list(app: &App, cat: &Catalog) -> CmdResult<Vec<ModelHero>> {
    let d = game_index(app)?;
    let e = d.find("scripts/heroes.vdata_c").ok_or("The game's hero list (scripts/heroes.vdata) is missing")?;
    let kv = modcore::kv3::parse_resource(&d.extract(e).map_err(err)?).map_err(err)?;
    let models = model::hero_models(&kv);
    Ok(cat
        .heroes
        .iter()
        .filter_map(|h| {
            models.iter().find(|m| m.code == h.code).map(|m| ModelHero { code: h.code.clone(), name: h.name.clone(), model: m.model.clone() })
        })
        .collect())
}

/// Heroes whose model can be replaced, with their model paths.
#[tauri::command]
async fn model_heroes(app: State<'_, App>) -> CmdResult<Vec<ModelHero>> {
    let cat = catalog_of(&app).await?;
    hero_model_list(&app, &cat)
}

#[tauri::command]
async fn create_model_project(app: State<'_, App>, name: String, hero: String) -> CmdResult<ModelProject> {
    let cat = catalog_of(&app).await?;
    let heroes = hero_model_list(&app, &cat)?;
    let h = heroes.iter().find(|h| h.code == hero).ok_or("That hero's model wasn't found in the game files")?;
    let mut lib = app.lib.lock().unwrap();
    let p = lib.create_model_project(&name, &h.code, &h.model).map_err(err)?;
    lib.log(&format!("Created model project \"{}\" for {}", p.name, h.name));
    Ok(p)
}

#[tauri::command]
fn rename_model_project(app: State<App>, id: String, name: String) -> CmdResult<()> {
    app.lib.lock().unwrap().rename_model_project(&id, &name).map_err(err)
}

#[tauri::command]
fn delete_model_project(app: State<App>, id: String) -> CmdResult<()> {
    let mut lib = app.lib.lock().unwrap();
    let name = lib.model_project(&id).map(|p| p.name.clone()).unwrap_or_default();
    lib.delete_model_project(&id).map_err(err)?;
    lib.log(&format!("Deleted model project \"{name}\""));
    Ok(())
}

/// What the extracted original contains (saved at extract time).
#[tauri::command]
fn model_summary(app: State<App>, id: String) -> CmdResult<Option<ModelSummary>> {
    let lib = app.lib.lock().unwrap();
    let p = lib.model_dir(&id).join("summary.json");
    match std::fs::read(&p) {
        Ok(b) => serde_json::from_slice(&b).map(Some).map_err(|e| e.to_string()),
        Err(_) => Ok(None),
    }
}

#[tauri::command]
fn open_model_folder(app_handle: tauri::AppHandle, app: State<App>, id: String, which: String) -> CmdResult<()> {
    let lib = app.lib.lock().unwrap();
    let base = lib.model_dir(&id);
    let path = match which.as_str() {
        "blender" => base.join("blender"),
        "return" => base.join("return"),
        "original" => base.join("original"),
        _ => base,
    };
    if !path.exists() {
        return Err("Extract the hero first".into());
    }
    app_handle.opener().open_path(path.display().to_string(), None::<&str>).map_err(|e| e.to_string())
}

/// Download (once) and return Source 2 Viewer's command-line tool.
fn ensure_s2v(tools_dir: &Path, progress: &dyn Fn(&str)) -> CmdResult<PathBuf> {
    if let Some(p) = tools::s2v_exe(tools_dir) {
        return Ok(p);
    }
    progress("Downloading Source 2 Viewer (53 MB, first time only)");
    let resp = ureq::get(model::S2V_URL)
        .call()
        .map_err(|e| format!("Couldn't download Source 2 Viewer: {e}"))?;
    let total: usize = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut reader = resp.into_reader();
    let mut bytes = Vec::with_capacity(total);
    let mut buf = vec![0u8; 1 << 16];
    let mut last = 0;
    loop {
        let n = std::io::Read::read(&mut reader, &mut buf).map_err(|e| format!("Download interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        if let Some(pct) = (bytes.len() * 100).checked_div(total) {
            if pct >= last + 5 {
                last = pct;
                progress(&format!("Downloading Source 2 Viewer: {pct}%"));
            }
        }
    }
    progress("Installing Source 2 Viewer");
    tools::install_s2v(&bytes, tools_dir).map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtractResult {
    report: ExtractReport,
    project: ModelProject,
}

/// Take the project's hero apart: model source, meshes, Blender file.
/// Progress goes out as `model-progress` events.
#[tauri::command]
async fn extract_model_project(app_handle: tauri::AppHandle, app: State<'_, App>, id: String) -> CmdResult<ExtractResult> {
    let (project, game_dir, dir, tools_dir) = {
        let lib = app.lib.lock().unwrap();
        let p = lib.model_project(&id).map_err(err)?.clone();
        (p, require_game_dir(&lib)?, lib.model_dir(&id), lib.root.join("tools"))
    };
    let h = app_handle.clone();
    let model_path = project.model.clone();
    let gd = game_dir.clone();
    let report = tauri::async_runtime::spawn_blocking(move || -> CmdResult<ExtractReport> {
        let say = |m: &str| {
            let _ = h.emit("model-progress", m.to_string());
        };
        let s2v = ensure_s2v(&tools_dir, &say)?;
        model::extract(&s2v, &gd, &model_path, &dir, &model::run_tool, &say).map_err(err)
    })
    .await
    .map_err(|e| e.to_string())??;
    let mut lib = app.lib.lock().unwrap();
    lib.mark_extracted(&id, steam::build_id(&game_dir)).map_err(err)?;
    lib.log(&format!(
        "Extracted {} for \"{}\": {} bones, {} meshes, {} materials",
        project.model,
        project.name,
        report.summary.bones.len(),
        report.summary.meshes,
        report.summary.materials.len()
    ));
    let project = lib.model_project(&id).map_err(err)?.clone();
    Ok(ExtractResult { report, project })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelBuildResult {
    report: ModelBuildReport,
    mod_id: String,
    mod_name: String,
}

/// Rebuild the project's model with CSDK and add it to the library.
#[tauri::command]
async fn build_model_project(app_handle: tauri::AppHandle, app: State<'_, App>, id: String) -> CmdResult<ModelBuildResult> {
    let (project, csdk_dir, game_dir, dir, out) = {
        let lib = app.lib.lock().unwrap();
        let p = lib.model_project(&id).map_err(err)?.clone();
        let c = lib.state.settings.csdk_dir.clone().ok_or("Set the CSDK folder in Settings first")?;
        let safe: String = p.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        let out = lib.root.join("builds").join(format!("{}_{}.vpk", safe.trim_matches('_'), &p.id[..4]));
        (p.clone(), PathBuf::from(c), require_game_dir(&lib)?, lib.model_dir(&id), out)
    };
    if project.extracted.is_none() {
        return Err("Extract the hero first".into());
    }
    if !steam_running() {
        return Err("Start Steam first: CSDK's compiler needs it running.".into());
    }
    let slug = format!("{}_{}", project.name, &project.id[..4]);
    let h = app_handle.clone();
    let model_path = project.model.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        model::build(&csdk_dir, &game_dir, &model_path, &dir, &slug, &out, &csdk::run_compiler, &|m| {
            let _ = h.emit("model-progress", m.to_string());
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(err)?;
    let mut lib = app.lib.lock().unwrap();
    let mod_id = lib.store_model_build(&id, Path::new(&report.vpk), "Built in Model Builder").map_err(err)?;
    let mod_name = lib.get(&mod_id).map(|m| m.name.clone()).unwrap_or_default();
    lib.log(&format!("Built model mod \"{}\" ({} files, {} warning(s))", mod_name, report.files.len(), report.warnings.len()));
    Ok(ModelBuildResult { report, mod_id, mod_name })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let mut lib = Library::open(&data)?;
            if lib.state.settings.game_dir.is_none() {
                if let Some(found) = steam::detect_game_dir() {
                    lib.state.settings.game_dir = Some(found.display().to_string());
                    lib.log(&format!("Found Deadlock at {}", found.display()));
                    lib.save()?;
                }
            }
            if lib.state.settings.csdk_dir.is_none() {
                if let Some(found) = csdk::detect() {
                    lib.state.settings.csdk_dir = Some(found.display().to_string());
                    lib.log(&format!("Found CSDK at {}", found.display()));
                    lib.save()?;
                }
            }
            app.manage(App { lib: Mutex::new(lib), game_index: Mutex::new(None), catalog: Mutex::new(None) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_overview,
            check_for_update,
            dismiss_safe_mode,
            detect_game_dir,
            set_game_dir,
            import_mods,
            replace_mod_file,
            update_mod,
            remove_mod,
            set_enabled,
            set_all_enabled,
            reorder,
            mark_verified,
            save_profile,
            delete_profile,
            apply_profile,
            apply_changes,
            restore_vanilla,
            read_log,
            open_folder,
            launch_game,
            search_game_sounds,
            audio_level,
            audio_bytes,
            detect_csdk,
            set_csdk_dir,
            create_sound_project,
            update_sound_project,
            delete_sound_project,
            set_project_item,
            remove_project_item,
            build_sound_project,
            set_item_volume,
            set_voice_mute,
            sound_heroes,
            describe_sounds,
            model_heroes,
            create_model_project,
            rename_model_project,
            delete_model_project,
            model_summary,
            open_model_folder,
            extract_model_project,
            build_model_project,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Deadlock Mod Studio");
}
