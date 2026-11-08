//! Tauri commands. All file logic lives in `modcore`; this layer adds the
//! game-running guard, picks the data folder and talks to the UI.

use modcore::audio::{self, Level};
use modcore::csdk::{self, BuildReport};
use modcore::library::{BuildCheck, Library, ModEntry, ModMeta, Profile, SafeModeEvent, Settings, SoundProject};
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SoundSearch {
    total: usize,
    results: Vec<GameSound>,
}

/// Every word in `query` must appear in the path. Max `limit` results.
#[tauri::command]
async fn search_game_sounds(app: State<'_, App>, query: String, limit: usize) -> CmdResult<SoundSearch> {
    let d = game_index(&app)?;
    let words: Vec<String> = query
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || c == '/' || c == '\\')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect();
    let mut total = 0;
    let mut results = Vec::new();
    for e in d.entries.iter().filter(|e| e.path.starts_with("sounds/") && e.path.ends_with(".vsnd_c")) {
        if words.iter().all(|w| e.path.contains(w.as_str())) {
            total += 1;
            if results.len() < limit.min(500) {
                results.push(GameSound { path: e.path.clone(), size: e.size(), looped: csdk::is_loop(&e.path) });
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
        let out = lib.root.join("builds").join(format!("{}.vpk", p.id));
        (p, PathBuf::from(c), g, out)
    };
    if !steam_running() {
        return Err("Start Steam first: CSDK's compiler needs it running.".into());
    }
    let slug = format!("{}_{}", project.name, &project.id[..4]);
    let jobs = project.items.clone();
    let h = app_handle.clone();
    let report = tauri::async_runtime::spawn_blocking(move || {
        csdk::build_sound_mod(&csdk_dir, &game_dir, &slug, &jobs, &out, &csdk::run_compiler, &|m| {
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
        "Built sound mod \"{}\": {} sound(s), {} turned down to match the originals",
        mod_name,
        report.items.len(),
        report.items.iter().filter(|i| i.gain_db < 0.0).count()
    ));
    Ok(BuildResult { report, mod_id, mod_name })
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
            app.manage(App { lib: Mutex::new(lib), game_index: Mutex::new(None) });
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Deadlock Mod Studio");
}
