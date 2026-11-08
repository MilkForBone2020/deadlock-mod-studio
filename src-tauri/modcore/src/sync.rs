//! Making the game folder match the library.
//!
//! `plan` works out what would change and touches nothing (the preview).
//! `apply` carries the plan out. Only files listed in `state.managed` are
//! ever removed; anything else in `addons` is left alone.

use crate::library::{Library, ManagedFile};
use crate::{gameinfo, steam, Error, IoCtx, Result};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::Path;

pub const MAX_MODS: usize = 99;
const BACKUPS_KEPT: usize = 10;

pub fn pak_name(index: usize) -> String {
    format!("pak{:02}_dir.vpk", index + 1)
}

fn is_pak_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.len() == "pak01_dir.vpk".len()
        && n.starts_with("pak")
        && n.ends_with("_dir.vpk")
        && n[3..5].chars().all(|c| c.is_ascii_digit())
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Action {
    CreateAddonsDir,
    #[serde(rename_all = "camelCase")]
    Remove { file: String, mod_name: Option<String> },
    #[serde(rename_all = "camelCase")]
    Copy { file: String, mod_id: String, mod_name: String },
    AddSearchPath,
    RemoveSearchPath,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub actions: Vec<Action>,
    /// Things worth knowing that don't stop the apply.
    pub warnings: Vec<String>,
    /// Problems that stop the apply until fixed.
    pub blockers: Vec<String>,
    /// Final layout: file name -> mod name, in load order.
    pub layout: Vec<(String, String)>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    pub done: Vec<String>,
    pub backup: Option<String>,
}

fn addons_listing(game_dir: &Path) -> Result<Vec<String>> {
    let dir = steam::addons_dir(game_dir);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for e in std::fs::read_dir(&dir).at(&dir)? {
        let e = e.at(&dir)?;
        if e.file_type().at(&dir)?.is_file() {
            out.push(e.file_name().to_string_lossy().into_owned());
        }
    }
    out.sort();
    Ok(out)
}

/// Work out what applying the library would change. Reads only.
pub fn plan(lib: &Library, game_dir: &Path) -> Result<Plan> {
    let mut p = Plan::default();
    if !steam::is_game_dir(game_dir) {
        p.blockers.push(format!(
            "No Deadlock install at {} (game\\citadel\\gameinfo.gi is missing)",
            game_dir.display()
        ));
        return Ok(p);
    }
    let enabled: Vec<_> = lib.state.mods.iter().filter(|m| m.enabled).collect();
    if enabled.len() > MAX_MODS {
        p.blockers.push(format!("Deadlock loads at most {MAX_MODS} addon VPKs; {} are on", enabled.len()));
        return Ok(p);
    }
    let addons = steam::addons_dir(game_dir);
    let on_disk: BTreeSet<String> =
        addons_listing(game_dir)?.into_iter().map(|n| n.to_ascii_lowercase()).collect();
    let managed_names: BTreeSet<String> =
        lib.state.managed.iter().map(|m| m.name.to_ascii_lowercase()).collect();

    let desired: Vec<(String, &crate::library::ModEntry)> =
        enabled.iter().enumerate().map(|(i, m)| (pak_name(i), *m)).collect();
    p.layout = desired.iter().map(|(n, m)| (n.clone(), m.name.clone())).collect();

    if !desired.is_empty() && !addons.exists() {
        p.actions.push(Action::CreateAddonsDir);
    }

    // Which managed files can stay exactly as they are?
    let mut kept: BTreeSet<String> = BTreeSet::new();
    for mf in &lib.state.managed {
        let lname = mf.name.to_ascii_lowercase();
        let present = on_disk.contains(&lname);
        let still_wanted = desired.iter().any(|(n, m)| {
            n.eq_ignore_ascii_case(&mf.name) && m.id == mf.mod_id && m.stamp == mf.stamp
        });
        let size_ok = present
            && std::fs::metadata(addons.join(&mf.name)).map(|md| md.len() == mf.size).unwrap_or(false);
        if still_wanted && size_ok {
            kept.insert(lname);
        } else if present {
            let mod_name = lib.state.mods.iter().find(|m| m.id == mf.mod_id).map(|m| m.name.clone());
            p.actions.push(Action::Remove { file: mf.name.clone(), mod_name });
        }
    }

    for (name, m) in &desired {
        let lname = name.to_ascii_lowercase();
        if kept.contains(&lname) {
            continue;
        }
        if on_disk.contains(&lname) && !managed_names.contains(&lname) {
            p.blockers.push(format!(
                "addons_studio\\{name} is already there and wasn't put there by this app. Move it out of the addons_studio folder (or import it into the library) and try again."
            ));
            continue;
        }
        if !lib.stored_path(m).exists() {
            p.blockers.push(format!("The library copy of \"{}\" is missing", m.name));
            continue;
        }
        p.actions.push(Action::Copy { file: name.clone(), mod_id: m.id.clone(), mod_name: m.name.clone() });
    }

    let foreign: Vec<&String> =
        on_disk.iter().filter(|n| is_pak_name(n) && !managed_names.contains(*n)).collect();
    if !foreign.is_empty() {
        p.warnings.push(format!(
            "{} VPK(s) in addons_studio weren't placed by this app and will also load: {}",
            foreign.len(),
            foreign.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }

    let gi_text = std::fs::read_to_string(steam::gameinfo_path(game_dir))
        .at(&steam::gameinfo_path(game_dir))?;
    match gameinfo::has_addons_path(&gi_text) {
        Ok(has) => {
            if !desired.is_empty() && !has {
                p.actions.push(Action::AddSearchPath);
            } else if desired.is_empty() && has {
                p.actions.push(Action::RemoveSearchPath);
            }
        }
        Err(e) => p.blockers.push(format!("gameinfo.gi can't be read safely: {e}")),
    }

    let conflicts = lib.conflicts();
    if !conflicts.is_empty() {
        p.warnings.push(format!(
            "{} internal file(s) are changed by more than one enabled mod",
            conflicts.len()
        ));
    }
    Ok(p)
}

fn backup_gameinfo(lib: &Library, game_dir: &Path) -> Result<String> {
    let src = steam::gameinfo_path(game_dir);
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = lib.backups_dir().join(format!("gameinfo-{stamp}.gi"));
    std::fs::copy(&src, &dest).at(&dest)?;
    // Keep only the newest backups.
    let mut all: Vec<_> = std::fs::read_dir(lib.backups_dir())
        .at(&lib.backups_dir())?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with("gameinfo-"))
                .unwrap_or(false)
        })
        .collect();
    all.sort();
    while all.len() > BACKUPS_KEPT {
        let _ = std::fs::remove_file(all.remove(0));
    }
    Ok(dest.display().to_string())
}

/// Write gameinfo.gi safely: temp file, then swap in. Keeps the read-only
/// attribute if the file had it.
fn write_gameinfo(path: &Path, text: &str) -> Result<()> {
    gameinfo::validate(text)?;
    let meta = std::fs::metadata(path).at(path)?;
    let was_readonly = meta.permissions().readonly();
    if was_readonly {
        let mut perm = meta.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        std::fs::set_permissions(path, perm).at(path)?;
    }
    let tmp = path.with_extension("gi.tmp");
    std::fs::write(&tmp, text).at(&tmp)?;
    std::fs::rename(&tmp, path).at(path)?;
    if was_readonly {
        let mut perm = std::fs::metadata(path).at(path)?.permissions();
        perm.set_readonly(true);
        std::fs::set_permissions(path, perm).at(path)?;
    }
    Ok(())
}

/// Carry out the plan. Re-plans first so it never acts on a stale preview.
/// The caller must make sure Deadlock isn't running.
pub fn apply(lib: &mut Library, game_dir: &Path) -> Result<ApplyReport> {
    let p = plan(lib, game_dir)?;
    if !p.blockers.is_empty() {
        return Err(Error::msg(p.blockers.join("\n")));
    }
    let addons = steam::addons_dir(game_dir);
    let mut report = ApplyReport { done: Vec::new(), backup: None };
    let result = (|| -> Result<()> {
        for a in &p.actions {
            match a {
                Action::CreateAddonsDir => {
                    std::fs::create_dir_all(&addons).at(&addons)?;
                    report.done.push("Created the addons_studio folder".into());
                }
                Action::Remove { file, mod_name } => {
                    let path = addons.join(file);
                    std::fs::remove_file(&path).at(&path)?;
                    lib.state.managed.retain(|m| !m.name.eq_ignore_ascii_case(file));
                    let what = mod_name.clone().unwrap_or_else(|| "deleted mod".into());
                    report.done.push(format!("Removed addons_studio\\{file} ({what})"));
                }
                _ => {}
            }
        }
        // Drop records for managed files that vanished on their own.
        lib.state.managed.retain(|m| addons.join(&m.name).exists());
        for a in &p.actions {
            if let Action::Copy { file, mod_id, mod_name } = a {
                let m = lib.get(mod_id)?.clone();
                let src = lib.stored_path(&m);
                let dest = addons.join(file);
                let tmp = addons.join(format!("{file}.tmp"));
                std::fs::copy(&src, &tmp).at(&tmp)?;
                std::fs::rename(&tmp, &dest).at(&dest)?;
                lib.state.managed.retain(|x| !x.name.eq_ignore_ascii_case(file));
                lib.state.managed.push(ManagedFile {
                    name: file.clone(),
                    mod_id: m.id.clone(),
                    stamp: m.stamp.clone(),
                    size: m.size,
                });
                report.done.push(format!("Placed \"{mod_name}\" as addons_studio\\{file}"));
            }
        }
        let gi = steam::gameinfo_path(game_dir);
        for a in &p.actions {
            let edit: Option<fn(&str) -> Result<String>> = match a {
                Action::AddSearchPath => Some(gameinfo::add_addons_path),
                Action::RemoveSearchPath => Some(gameinfo::remove_addons_path),
                _ => None,
            };
            if let Some(edit) = edit {
                let text = std::fs::read_to_string(&gi).at(&gi)?;
                let new_text = edit(&text)?;
                if new_text != text {
                    report.backup = Some(backup_gameinfo(lib, game_dir)?);
                    write_gameinfo(&gi, &new_text)?;
                    report.done.push(if matches!(a, Action::AddSearchPath) {
                        "Added the addons_studio line to gameinfo.gi".into()
                    } else {
                        "Removed the addons_studio line from gameinfo.gi".into()
                    });
                }
            }
        }
        Ok(())
    })();
    for line in &report.done {
        lib.log(line);
    }
    if let Err(e) = &result {
        lib.log(&format!("Apply stopped: {e}"));
    }
    lib.save()?;
    result.map(|_| report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::ModMeta;
    use crate::vpk;
    use std::path::PathBuf;

    const GI: &str = "GameInfo\r\n{\r\n\tFileSystem\r\n\t{\r\n\t\tSearchPaths\r\n\t\t{\r\n\t\t\tMod\t\tcitadel\r\n\t\t\tGame\t\tcitadel\r\n\t\t\tGame\t\tcore\r\n\t\t}\r\n\t}\r\n}\r\n";

    struct Fx {
        _t: tempfile::TempDir,
        game: PathBuf,
        lib: Library,
        root: PathBuf,
    }

    fn fx(n: usize) -> Fx {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().to_path_buf();
        let game = root.join("steamapps/common/Deadlock");
        std::fs::create_dir_all(steam::citadel_dir(&game)).unwrap();
        std::fs::write(steam::gameinfo_path(&game), GI).unwrap();
        let mut lib = Library::open(&root.join("data")).unwrap();
        for i in 0..n {
            let src = root.join(format!("m{i}.vpk"));
            std::fs::write(&src, vpk::build_test_vpk(&[&format!("sounds/s{i}.vsnd_c")])).unwrap();
            lib.import_vpk(&src, ModMeta::default()).unwrap();
        }
        Fx { _t: t, game, lib, root }
    }

    fn gi(f: &Fx) -> String {
        std::fs::read_to_string(steam::gameinfo_path(&f.game)).unwrap()
    }

    fn addons(f: &Fx) -> Vec<String> {
        addons_listing(&f.game).unwrap()
    }

    #[test]
    fn nothing_enabled_means_nothing_to_do() {
        let f = fx(2);
        let p = plan(&f.lib, &f.game).unwrap();
        assert!(p.is_empty(), "{p:?}");
        assert!(p.blockers.is_empty());
    }

    #[test]
    fn enable_apply_then_restore_vanilla() {
        let mut f = fx(3);
        let ids: Vec<String> = f.lib.state.mods.iter().map(|m| m.id.clone()).collect();
        f.lib.set_enabled(&ids[0], true).unwrap();
        f.lib.set_enabled(&ids[2], true).unwrap();

        let p = plan(&f.lib, &f.game).unwrap();
        assert_eq!(p.actions.len(), 4); // create dir, 2 copies, add line
        let r = apply(&mut f.lib, &f.game).unwrap();
        assert!(r.backup.is_some());
        assert_eq!(addons(&f), vec!["pak01_dir.vpk", "pak02_dir.vpk"]);
        assert!(gameinfo::has_addons_path(&gi(&f)).unwrap());
        // pak02 is mod 2's file
        let pak02 = vpk::list_files(&steam::addons_dir(&f.game).join("pak02_dir.vpk")).unwrap();
        assert_eq!(pak02, vec!["sounds/s2.vsnd_c"]);

        // Second plan: nothing to do.
        assert!(plan(&f.lib, &f.game).unwrap().is_empty());

        // Restore vanilla: everything off.
        f.lib.set_all_enabled(false).unwrap();
        apply(&mut f.lib, &f.game).unwrap();
        assert!(addons(&f).is_empty());
        assert_eq!(gi(&f), GI, "gameinfo.gi must be byte-identical to Valve's");
        assert!(f.lib.state.managed.is_empty());
    }

    #[test]
    fn reorder_renumbers_only_what_changed() {
        let mut f = fx(2);
        f.lib.set_all_enabled(true).unwrap();
        apply(&mut f.lib, &f.game).unwrap();
        let ids: Vec<String> = f.lib.state.mods.iter().rev().map(|m| m.id.clone()).collect();
        f.lib.reorder(&ids).unwrap();
        let p = plan(&f.lib, &f.game).unwrap();
        // both files swap: 2 removes + 2 copies, gameinfo already fine
        assert_eq!(p.actions.len(), 4, "{:?}", p.actions);
        apply(&mut f.lib, &f.game).unwrap();
        let pak01 = vpk::list_files(&steam::addons_dir(&f.game).join("pak01_dir.vpk")).unwrap();
        assert_eq!(pak01, vec!["sounds/s1.vsnd_c"]);
    }

    #[test]
    fn foreign_files_block_and_are_never_deleted() {
        let mut f = fx(1);
        let addons_dir = steam::addons_dir(&f.game);
        std::fs::create_dir_all(&addons_dir).unwrap();
        std::fs::write(addons_dir.join("pak01_dir.vpk"), b"someone else's").unwrap();
        f.lib.set_all_enabled(true).unwrap();
        let p = plan(&f.lib, &f.game).unwrap();
        assert_eq!(p.blockers.len(), 1);
        assert!(apply(&mut f.lib, &f.game).is_err());
        assert_eq!(std::fs::read(addons_dir.join("pak01_dir.vpk")).unwrap(), b"someone else's");
        assert_eq!(gi(&f), GI);
    }

    #[test]
    fn foreign_file_elsewhere_is_a_warning_only() {
        let mut f = fx(1);
        let addons_dir = steam::addons_dir(&f.game);
        std::fs::create_dir_all(&addons_dir).unwrap();
        std::fs::write(addons_dir.join("pak50_dir.vpk"), b"x").unwrap();
        f.lib.set_all_enabled(true).unwrap();
        let p = plan(&f.lib, &f.game).unwrap();
        assert!(p.blockers.is_empty());
        assert_eq!(p.warnings.len(), 1);
        apply(&mut f.lib, &f.game).unwrap();
        f.lib.set_all_enabled(false).unwrap();
        apply(&mut f.lib, &f.game).unwrap();
        assert_eq!(addons(&f), vec!["pak50_dir.vpk"]);
    }

    #[test]
    fn steam_reset_is_detected_and_fixed() {
        let mut f = fx(1);
        f.lib.set_all_enabled(true).unwrap();
        apply(&mut f.lib, &f.game).unwrap();
        // Steam update / verify puts Valve's file back.
        std::fs::write(steam::gameinfo_path(&f.game), GI).unwrap();
        let p = plan(&f.lib, &f.game).unwrap();
        assert_eq!(p.actions, vec![Action::AddSearchPath]);
    }

    #[test]
    fn safe_mode_then_apply_clears_addons() {
        let mut f = fx(2);
        f.lib.set_all_enabled(true).unwrap();
        f.lib.check_build(Some("1")).unwrap();
        apply(&mut f.lib, &f.game).unwrap();
        f.lib.check_build(Some("2")).unwrap();
        apply(&mut f.lib, &f.game).unwrap();
        assert!(addons(&f).is_empty());
        assert_eq!(gi(&f), GI);
    }

    #[test]
    fn replaced_vpk_is_recopied() {
        let mut f = fx(1);
        f.lib.set_all_enabled(true).unwrap();
        apply(&mut f.lib, &f.game).unwrap();
        let id = f.lib.state.mods[0].id.clone();
        let src = f.root.join("new.vpk");
        std::fs::write(&src, vpk::build_test_vpk(&["sounds/new.vsnd_c"])).unwrap();
        f.lib.replace_vpk(&id, &src).unwrap();
        let p = plan(&f.lib, &f.game).unwrap();
        assert_eq!(p.actions.len(), 2); // remove old copy, copy new
        apply(&mut f.lib, &f.game).unwrap();
        let pak = vpk::list_files(&steam::addons_dir(&f.game).join("pak01_dir.vpk")).unwrap();
        assert_eq!(pak, vec!["sounds/new.vsnd_c"]);
    }

    #[test]
    fn missing_game_is_a_blocker() {
        let f = fx(0);
        let p = plan(&f.lib, &f.root.join("nope")).unwrap();
        assert_eq!(p.blockers.len(), 1);
    }

    #[test]
    fn backups_are_capped() {
        let f = fx(0);
        for i in 0..12 {
            std::fs::write(f.lib.backups_dir().join(format!("gameinfo-20200101-0000{i:02}.gi")), "old").unwrap();
        }
        let newest = backup_gameinfo(&f.lib, &f.game).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(f.lib.backups_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), BACKUPS_KEPT);
        assert!(newest.ends_with(names.last().unwrap()));
        assert_eq!(std::fs::read_to_string(&newest).unwrap(), GI);
    }

    #[test]
    fn other_managers_folder_and_line_are_left_alone() {
        let mut f = fx(1);
        let dmm_gi = GI.replace("\t\t\tGame\t\tcitadel\r\n", "\t\t\tGame\t\tcitadel/addons\r\n\t\t\tGame\t\tcitadel\r\n");
        std::fs::write(steam::gameinfo_path(&f.game), &dmm_gi).unwrap();
        let theirs = steam::citadel_dir(&f.game).join("addons");
        std::fs::create_dir_all(&theirs).unwrap();
        std::fs::write(theirs.join("pak01_dir.vpk"), b"dmm mod").unwrap();

        // Nothing on: nothing to do, even though their line is present.
        assert!(plan(&f.lib, &f.game).unwrap().is_empty());

        f.lib.set_all_enabled(true).unwrap();
        apply(&mut f.lib, &f.game).unwrap();
        assert_eq!(addons(&f), vec!["pak01_dir.vpk"]);
        assert!(gi(&f).contains("citadel/addons_studio\r\n\t\t\tGame\t\tcitadel/addons\r\n"));

        f.lib.set_all_enabled(false).unwrap();
        apply(&mut f.lib, &f.game).unwrap();
        assert_eq!(gi(&f), dmm_gi, "their line must survive");
        assert_eq!(std::fs::read(theirs.join("pak01_dir.vpk")).unwrap(), b"dmm mod");
    }

    #[test]
    fn pak_names() {
        assert_eq!(pak_name(0), "pak01_dir.vpk");
        assert_eq!(pak_name(98), "pak99_dir.vpk");
        assert!(is_pak_name("PAK07_dir.vpk"));
        assert!(!is_pak_name("pak7_dir.vpk"));
    }
}
