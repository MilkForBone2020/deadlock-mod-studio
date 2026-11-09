//! The mod library: my own VPKs stored outside the game folder, plus the
//! saved state (order, on/off, profiles, what the app placed in `addons`).

use crate::{vpk, Error, IoCtx, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ModKind {
    Skin,
    Sound,
    Ui,
    #[default]
    Other,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: ModKind,
    #[serde(default)]
    pub hero: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub version: String,
    /// File name inside the library folder, e.g. `<id>.vpk`.
    pub stored_file: String,
    /// The name the VPK had when I imported it.
    pub original_name: String,
    pub size: u64,
    /// Changes every time the VPK is replaced, so sync knows to recopy it.
    pub stamp: String,
    pub added_at: String,
    pub updated_at: String,
    pub enabled: bool,
    /// The Deadlock build this mod was last tested and confirmed working on.
    pub verified_build: Option<String>,
    /// Internal paths the VPK overrides.
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    /// Enabled mods, in load order.
    pub mods: Vec<String>,
}

/// Mods grouped into one pack (e.g. a full Seven pack). The pack's switch
/// turns the whole pack on or off; each mod keeps its own switch. Members
/// always sit together in the load order.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pack {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub notes: String,
    pub enabled: bool,
    /// Member mod ids (their order is the library order).
    pub mods: Vec<String>,
    pub created: String,
}

/// `pack.json` inside an exported pack file.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct PackFile {
    format: u32,
    name: String,
    #[serde(default)]
    notes: String,
    mods: Vec<PackFileMod>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
struct PackFileMod {
    name: String,
    #[serde(default)]
    kind: ModKind,
    #[serde(default)]
    hero: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    original_name: String,
    /// Path of the VPK inside the pack file.
    file: String,
}

/// Exported packs: a zip with `pack.json` and the VPKs.
pub const PACK_EXT: &str = "dmpack";

/// A file the app placed in `addons`. Anything not listed here is never
/// touched.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ManagedFile {
    pub name: String,
    pub mod_id: String,
    pub stamp: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SafeModeEvent {
    pub from_build: String,
    pub to_build: String,
    pub at: String,
    /// Names of the mods that were switched off.
    pub disabled: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub game_dir: Option<String>,
    #[serde(default)]
    pub csdk_dir: Option<String>,
    #[serde(default)]
    pub blender_exe: Option<String>,
    /// "<add-on version>|<blender exe>" the add-on was last installed as.
    #[serde(default)]
    pub blender_addon: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VoiceMute {
    /// Hero code name, e.g. `gigawatt`.
    pub hero: String,
    /// Ability slot 1–4.
    pub slot: u8,
}

/// A sound mod being made in the app: which game sounds it replaces, with
/// which of my files. Building it produces (or updates) a library mod.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SoundProject {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub hero: String,
    #[serde(default)]
    pub items: Vec<crate::csdk::SoundJob>,
    /// Hero abilities whose voice lines this mod replaces with silence.
    #[serde(default)]
    pub mute_voice: Vec<VoiceMute>,
    /// The library mod this project builds into, once built.
    #[serde(default)]
    pub mod_id: Option<String>,
    #[serde(default)]
    pub last_built: Option<String>,
}

/// A hero model mod being made in the app. Its files live in
/// `models/<id>/` in the data folder (see `model::extract`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelProject {
    pub id: String,
    pub name: String,
    /// Hero code, e.g. `gigawatt`.
    pub hero: String,
    /// Game path of the model replaced, e.g. `models/heroes_staging/x/x.vmdl`.
    pub model: String,
    pub created: String,
    /// When the original was last extracted, and from which game build.
    #[serde(default)]
    pub extracted: Option<String>,
    #[serde(default)]
    pub extracted_build: Option<String>,
    /// Source 2 Viewer version used, so older extracts can be redone.
    #[serde(default)]
    pub extracted_tool: Option<String>,
    #[serde(default)]
    pub mod_id: Option<String>,
    #[serde(default)]
    pub last_built: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub schema: u32,
    #[serde(default)]
    pub settings: Settings,
    /// Library order is load order: index 0 becomes pak01_dir.vpk.
    #[serde(default)]
    pub mods: Vec<ModEntry>,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub managed: Vec<ManagedFile>,
    #[serde(default)]
    pub last_build_id: Option<String>,
    #[serde(default)]
    pub safe_mode: Option<SafeModeEvent>,
    #[serde(default)]
    pub sound_projects: Vec<SoundProject>,
    #[serde(default)]
    pub model_projects: Vec<ModelProject>,
    #[serde(default)]
    pub packs: Vec<Pack>,
}

impl Default for State {
    fn default() -> Self {
        State {
            schema: 1,
            settings: Settings::default(),
            mods: Vec::new(),
            profiles: Vec::new(),
            managed: Vec::new(),
            last_build_id: None,
            safe_mode: None,
            sound_projects: Vec::new(),
            model_projects: Vec::new(),
            packs: Vec::new(),
        }
    }
}

#[derive(Deserialize, Default, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModMeta {
    pub name: Option<String>,
    pub kind: Option<ModKind>,
    pub hero: Option<String>,
    pub notes: Option<String>,
    pub version: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub path: String,
    /// Mod ids in load order.
    pub mods: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub enum BuildCheck {
    /// Could not read Steam's manifest.
    Unknown,
    /// First time the app has seen a build: just remember it.
    FirstSeen,
    Same,
    /// New build: every mod was switched off.
    Changed(SafeModeEvent),
}

pub fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

pub struct Library {
    pub root: PathBuf,
    pub state: State,
}

impl Library {
    /// Open (or create) the library in the app's data folder.
    pub fn open(root: &Path) -> Result<Library> {
        std::fs::create_dir_all(root.join("library")).at(root)?;
        std::fs::create_dir_all(root.join("backups")).at(root)?;
        std::fs::create_dir_all(root.join("logs")).at(root)?;
        let path = root.join("library.json");
        let state = if path.exists() {
            let text = std::fs::read_to_string(&path).at(&path)?;
            serde_json::from_str(&text)?
        } else {
            State::default()
        };
        Ok(Library { root: root.to_path_buf(), state })
    }

    pub fn library_dir(&self) -> PathBuf {
        self.root.join("library")
    }
    pub fn backups_dir(&self) -> PathBuf {
        self.root.join("backups")
    }
    pub fn log_path(&self) -> PathBuf {
        self.root.join("logs").join("actions.log")
    }
    pub fn stored_path(&self, m: &ModEntry) -> PathBuf {
        self.library_dir().join(&m.stored_file)
    }

    /// Save atomically, keeping the previous copy as library.json.bak.
    pub fn save(&self) -> Result<()> {
        let path = self.root.join("library.json");
        let tmp = self.root.join("library.json.tmp");
        let text = serde_json::to_string_pretty(&self.state)?;
        std::fs::write(&tmp, text).at(&tmp)?;
        if path.exists() {
            let _ = std::fs::copy(&path, self.root.join("library.json.bak"));
        }
        std::fs::rename(&tmp, &path).at(&path)?;
        Ok(())
    }

    /// Append one line to the action log. Logging never fails an action.
    pub fn log(&self, msg: &str) {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())
        {
            let _ = writeln!(f, "{}  {}", now(), msg);
        }
    }

    pub fn get(&self, id: &str) -> Result<&ModEntry> {
        self.state
            .mods
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| Error::msg("That mod is no longer in the library"))
    }

    fn get_mut(&mut self, id: &str) -> Result<&mut ModEntry> {
        self.state
            .mods
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or_else(|| Error::msg("That mod is no longer in the library"))
    }

    /// Copy a VPK into the library. New mods are added at the bottom, off.
    pub fn import_vpk(&mut self, src: &Path, meta: ModMeta) -> Result<ModEntry> {
        if !vpk::is_vpk(src)? {
            return Err(Error::msg(format!(
                "{} is not a VPK file",
                src.file_name().unwrap_or_default().to_string_lossy()
            )));
        }
        let files = vpk::list_files(src)?;
        let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
        let stored_file = format!("{id}.vpk");
        let dest = self.library_dir().join(&stored_file);
        std::fs::copy(src, &dest).at(&dest)?;
        let size = std::fs::metadata(&dest).at(&dest)?.len();
        let original_name = src.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let default_name = original_name.trim_end_matches(".vpk").replace(['_', '-'], " ");
        let t = now();
        let entry = ModEntry {
            id: id.clone(),
            name: meta.name.filter(|s| !s.trim().is_empty()).unwrap_or(default_name),
            kind: meta.kind.unwrap_or_else(|| guess_kind(&files)),
            hero: meta.hero.unwrap_or_default(),
            notes: meta.notes.unwrap_or_default(),
            version: meta.version.unwrap_or_else(|| "1".into()),
            stored_file,
            original_name,
            size,
            stamp: uuid::Uuid::new_v4().simple().to_string(),
            added_at: t.clone(),
            updated_at: t,
            enabled: false,
            verified_build: None,
            files,
        };
        self.state.mods.push(entry.clone());
        self.log(&format!("Imported \"{}\" ({} files inside)", entry.name, entry.files.len()));
        self.save()?;
        Ok(entry)
    }

    /// Swap in a rebuilt VPK for an existing mod. Settings and order stay;
    /// the "verified" mark is cleared because it's a new build of the mod.
    pub fn replace_vpk(&mut self, id: &str, src: &Path) -> Result<()> {
        if !vpk::is_vpk(src)? {
            return Err(Error::msg("That file is not a VPK"));
        }
        let files = vpk::list_files(src)?;
        let dest = self.stored_path(self.get(id)?);
        std::fs::copy(src, &dest).at(&dest)?;
        let size = std::fs::metadata(&dest).at(&dest)?.len();
        let m = self.get_mut(id)?;
        m.files = files;
        m.size = size;
        m.stamp = uuid::Uuid::new_v4().simple().to_string();
        m.updated_at = now();
        m.verified_build = None;
        let name = m.name.clone();
        self.log(&format!("Replaced the VPK for \"{name}\""));
        self.save()
    }

    pub fn update_meta(&mut self, id: &str, meta: ModMeta) -> Result<()> {
        let m = self.get_mut(id)?;
        if let Some(n) = meta.name.filter(|s| !s.trim().is_empty()) {
            m.name = n;
        }
        if let Some(k) = meta.kind {
            m.kind = k;
        }
        if let Some(h) = meta.hero {
            m.hero = h;
        }
        if let Some(n) = meta.notes {
            m.notes = n;
        }
        if let Some(v) = meta.version {
            m.version = v;
        }
        m.updated_at = now();
        self.save()
    }

    /// Delete a mod from the library. Its copy in `addons` (if any) is
    /// removed on the next apply.
    pub fn remove_mod(&mut self, id: &str) -> Result<()> {
        let m = self.get(id)?.clone();
        let path = self.stored_path(&m);
        if path.exists() {
            std::fs::remove_file(&path).at(&path)?;
        }
        self.state.mods.retain(|x| x.id != id);
        for p in &mut self.state.profiles {
            p.mods.retain(|x| x != id);
        }
        self.group_packs();
        self.log(&format!("Removed \"{}\" from the library", m.name));
        self.save()
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<()> {
        self.get_mut(id)?.enabled = enabled;
        self.save()
    }

    pub fn set_all_enabled(&mut self, enabled: bool) -> Result<()> {
        for m in &mut self.state.mods {
            m.enabled = enabled;
        }
        self.save()
    }

    /// New load order. `ids` must contain every mod exactly once.
    pub fn reorder(&mut self, ids: &[String]) -> Result<()> {
        if ids.len() != self.state.mods.len() {
            return Err(Error::msg("Reorder list doesn't match the library"));
        }
        let mut by_id: BTreeMap<String, ModEntry> =
            self.state.mods.drain(..).map(|m| (m.id.clone(), m)).collect();
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            match by_id.remove(id) {
                Some(m) => out.push(m),
                None => {
                    // Put everything back before failing.
                    self.state.mods = out.into_iter().chain(by_id.into_values()).collect();
                    return Err(Error::msg("Reorder list has an unknown or repeated mod"));
                }
            }
        }
        self.state.mods = out;
        self.group_packs();
        self.save()
    }

    // ------------------------------------------------------------ packs ---

    pub fn pack_of(&self, mod_id: &str) -> Option<&Pack> {
        self.state.packs.iter().find(|p| p.mods.iter().any(|m| m == mod_id))
    }

    /// Will this mod load: switched on, and so is its pack (if any)?
    pub fn is_active(&self, m: &ModEntry) -> bool {
        m.enabled && self.pack_of(&m.id).map(|p| p.enabled).unwrap_or(true)
    }

    /// The mods that load, in load order.
    pub fn active_mods(&self) -> Vec<&ModEntry> {
        self.state.mods.iter().filter(|m| self.is_active(m)).collect()
    }

    fn pack_mut(&mut self, id: &str) -> Result<&mut Pack> {
        self.state.packs.iter_mut().find(|p| p.id == id).ok_or_else(|| Error::msg("That pack no longer exists"))
    }

    /// Keep packs tidy: drop missing mods and empty packs, list members in
    /// library order, and move each pack's mods together at the place of
    /// its first one.
    pub fn group_packs(&mut self) {
        let ids: Vec<String> = self.state.mods.iter().map(|m| m.id.clone()).collect();
        for p in &mut self.state.packs {
            p.mods = ids.iter().filter(|id| p.mods.contains(id)).cloned().collect();
        }
        self.state.packs.retain(|p| !p.mods.is_empty());
        let mut out: Vec<ModEntry> = Vec::with_capacity(self.state.mods.len());
        let mut placed: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mods = std::mem::take(&mut self.state.mods);
        for m in &mods {
            if placed.contains(&m.id) {
                continue;
            }
            match self.state.packs.iter().find(|p| p.mods.contains(&m.id)) {
                Some(p) => {
                    for id in &p.mods {
                        if let Some(x) = mods.iter().find(|x| &x.id == id) {
                            placed.insert(id.clone());
                            out.push(x.clone());
                        }
                    }
                }
                None => {
                    placed.insert(m.id.clone());
                    out.push(m.clone());
                }
            }
        }
        self.state.mods = out;
    }

    /// Group mods into a new pack (taking them out of any other pack).
    pub fn create_pack(&mut self, name: &str, mod_ids: &[String]) -> Result<Pack> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::msg("Give the pack a name"));
        }
        let ids: Vec<String> = mod_ids.iter().filter(|id| self.state.mods.iter().any(|m| &m.id == *id)).cloned().collect();
        if ids.is_empty() {
            return Err(Error::msg("Pick at least one mod for the pack"));
        }
        for p in &mut self.state.packs {
            p.mods.retain(|m| !ids.contains(m));
        }
        let pack = Pack { id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(), name: name.to_string(), notes: String::new(), enabled: true, mods: ids, created: now() };
        self.state.packs.push(pack.clone());
        self.group_packs();
        self.log(&format!("Made the pack \"{}\" ({} mods)", pack.name, pack.mods.len()));
        self.save()?;
        Ok(self.state.packs.iter().find(|p| p.id == pack.id).cloned().unwrap_or(pack))
    }

    pub fn update_pack(&mut self, id: &str, name: Option<String>, notes: Option<String>) -> Result<()> {
        let p = self.pack_mut(id)?;
        if let Some(n) = name.filter(|s| !s.trim().is_empty()) {
            p.name = n.trim().to_string();
        }
        if let Some(n) = notes {
            p.notes = n;
        }
        self.save()
    }

    pub fn set_pack_enabled(&mut self, id: &str, enabled: bool) -> Result<()> {
        self.pack_mut(id)?.enabled = enabled;
        self.save()
    }

    /// Put a mod in a pack (None: take it out of its pack).
    pub fn set_mod_pack(&mut self, mod_id: &str, pack_id: Option<&str>) -> Result<()> {
        self.get(mod_id)?;
        if let Some(pid) = pack_id {
            self.pack_mut(pid)?;
        }
        for p in &mut self.state.packs {
            p.mods.retain(|m| m != mod_id);
        }
        if let Some(pid) = pack_id {
            // Joins at the end of the pack.
            let p = self.pack_mut(pid)?;
            p.mods.push(mod_id.to_string());
            let last = p.mods.iter().rev().nth(1).cloned();
            if let Some(last) = last {
                let from = self.state.mods.iter().position(|m| m.id == mod_id).unwrap();
                let m = self.state.mods.remove(from);
                let at = self.state.mods.iter().position(|x| x.id == last).map(|i| i + 1).unwrap_or(self.state.mods.len());
                self.state.mods.insert(at, m);
            }
        }
        self.group_packs();
        self.save()
    }

    /// Remove a pack. `with_mods`: delete its mods from the library too;
    /// otherwise they stay as loose mods.
    pub fn delete_pack(&mut self, id: &str, with_mods: bool) -> Result<()> {
        let p = self.state.packs.iter().find(|p| p.id == id).cloned().ok_or_else(|| Error::msg("That pack no longer exists"))?;
        self.state.packs.retain(|x| x.id != id);
        if with_mods {
            for m in &p.mods {
                self.remove_mod(m)?;
            }
        }
        self.log(&format!("Removed the pack \"{}\"{}", p.name, if with_mods { " and its mods" } else { "" }));
        self.save()
    }

    /// Write a pack and its VPKs to one file.
    pub fn export_pack(&self, id: &str, dest: &Path) -> Result<()> {
        let p = self.state.packs.iter().find(|p| p.id == id).ok_or_else(|| Error::msg("That pack no longer exists"))?;
        let file = std::fs::File::create(dest).at(dest)?;
        let mut zip = zip::ZipWriter::new(file);
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(true);
        let mut entries = vec![];
        for (i, id) in p.mods.iter().enumerate() {
            let m = self.get(id)?;
            let safe: String = m.name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
            let inner = format!("mods/{:02}_{}.vpk", i + 1, safe.trim_matches('_'));
            let src = self.stored_path(m);
            zip.start_file(inner.as_str(), stored).map_err(|e| Error::msg(e.to_string()))?;
            let mut f = std::fs::File::open(&src).at(&src)?;
            std::io::copy(&mut f, &mut zip).at(dest)?;
            entries.push(PackFileMod { name: m.name.clone(), kind: m.kind, hero: m.hero.clone(), notes: m.notes.clone(), version: m.version.clone(), enabled: m.enabled, original_name: m.original_name.clone(), file: inner });
        }
        let json = serde_json::to_vec_pretty(&PackFile { format: 1, name: p.name.clone(), notes: p.notes.clone(), mods: entries })?;
        zip.start_file("pack.json", zip::write::SimpleFileOptions::default()).map_err(|e| Error::msg(e.to_string()))?;
        zip.write_all(&json).at(dest)?;
        zip.finish().map_err(|e| Error::msg(e.to_string()))?;
        self.log(&format!("Exported the pack \"{}\" to {}", p.name, dest.display()));
        Ok(())
    }

    /// Import a pack file: its mods join the library (off, at the bottom)
    /// as a new pack.
    pub fn import_pack(&mut self, src: &Path) -> Result<Pack> {
        let file = std::fs::File::open(src).at(src)?;
        let mut zip = zip::ZipArchive::new(file).map_err(|_| Error::msg("That isn't a pack file"))?;
        let pf: PackFile = {
            let mut e = zip.by_name("pack.json").map_err(|_| Error::msg("That isn't a pack file (no pack.json)"))?;
            let mut b = vec![];
            std::io::Read::read_to_end(&mut e, &mut b).at(src)?;
            serde_json::from_slice(&b).map_err(|e| Error::msg(format!("The pack file is damaged: {e}")))?
        };
        if pf.format > 1 {
            return Err(Error::msg("This pack was made by a newer version of the app. Update first."));
        }
        let tmp = self.root.join(format!("import_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]));
        std::fs::create_dir_all(&tmp).at(&tmp)?;
        let mut ids = vec![];
        let result = self.import_pack_mods(&mut zip, &pf, &tmp, &mut ids);
        let _ = std::fs::remove_dir_all(&tmp);
        if let Err(e) = result {
            // Nothing half-imported is left behind.
            for id in &ids {
                let _ = self.remove_mod(id);
            }
            return Err(e);
        }
        let mut pack = self.create_pack(&pf.name, &ids)?;
        // Arrives switched off, like any import.
        pack.enabled = false;
        pack.notes = pf.notes;
        let id = pack.id.clone();
        let p = self.pack_mut(&id)?;
        p.enabled = false;
        p.notes = pack.notes.clone();
        self.save()?;
        Ok(pack)
    }

    fn import_pack_mods(&mut self, zip: &mut zip::ZipArchive<std::fs::File>, pf: &PackFile, tmp: &Path, ids: &mut Vec<String>) -> Result<()> {
        for (i, pm) in pf.mods.iter().enumerate() {
            let out = tmp.join(format!("{i}.vpk"));
            {
                let mut e = zip.by_name(&pm.file).map_err(|_| Error::msg(format!("The pack is missing {}", pm.file)))?;
                let mut f = std::fs::File::create(&out).at(&out)?;
                std::io::copy(&mut e, &mut f).at(&out)?;
            }
            let meta = ModMeta { name: Some(pm.name.clone()), kind: Some(pm.kind), hero: Some(pm.hero.clone()), notes: Some(pm.notes.clone()), version: Some(pm.version.clone()) };
            let mut entry = self.import_vpk(&out, meta)?;
            if !pm.original_name.is_empty() {
                entry.original_name = pm.original_name.clone();
            }
            let m = self.get_mut(&entry.id)?;
            m.original_name = entry.original_name.clone();
            m.enabled = pm.enabled;
            let _ = std::fs::remove_file(&out);
            ids.push(entry.id);
        }
        Ok(())
    }

    pub fn mark_verified(&mut self, id: &str, build: Option<String>) -> Result<()> {
        self.get_mut(id)?.verified_build = build;
        self.save()
    }

    /// Save the currently enabled mods (in order) as a profile. Saving with
    /// an existing name overwrites that profile.
    pub fn save_profile(&mut self, name: &str) -> Result<Profile> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::msg("Give the profile a name"));
        }
        let mods: Vec<String> = self.active_mods().iter().map(|m| m.id.clone()).collect();
        let profile = if let Some(p) =
            self.state.profiles.iter_mut().find(|p| p.name.eq_ignore_ascii_case(name))
        {
            p.mods = mods;
            p.clone()
        } else {
            let p = Profile {
                id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
                name: name.to_string(),
                mods,
            };
            self.state.profiles.push(p.clone());
            p
        };
        self.save()?;
        Ok(profile)
    }

    pub fn delete_profile(&mut self, id: &str) -> Result<()> {
        self.state.profiles.retain(|p| p.id != id);
        self.save()
    }

    /// Turn on exactly the profile's mods, move them to the top in the
    /// profile's order, and leave everything else off below them.
    pub fn apply_profile(&mut self, id: &str) -> Result<()> {
        let p = self
            .state
            .profiles
            .iter()
            .find(|p| p.id == id)
            .cloned()
            .ok_or_else(|| Error::msg("That profile no longer exists"))?;
        let mut rest: Vec<ModEntry> = Vec::new();
        let mut by_id: BTreeMap<String, ModEntry> = BTreeMap::new();
        for m in self.state.mods.drain(..) {
            if p.mods.contains(&m.id) {
                by_id.insert(m.id.clone(), m);
            } else {
                rest.push(m);
            }
        }
        let mut out: Vec<ModEntry> = p
            .mods
            .iter()
            .filter_map(|id| by_id.remove(id))
            .map(|mut m| {
                m.enabled = true;
                m
            })
            .collect();
        for mut m in rest {
            m.enabled = false;
            out.push(m);
        }
        self.state.mods = out;
        // A pack with a mod in the profile is switched on (its other mods
        // are off by now).
        for pk in &mut self.state.packs {
            if pk.mods.iter().any(|id| p.mods.contains(id)) {
                pk.enabled = true;
            }
        }
        self.group_packs();
        self.save()
    }

    /// Internal files changed by more than one enabled mod.
    pub fn conflicts(&self) -> Vec<Conflict> {
        let mut map: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for m in self.active_mods() {
            for f in &m.files {
                map.entry(f.as_str()).or_default().push(m.id.clone());
            }
        }
        map.into_iter()
            .filter(|(_, v)| v.len() > 1)
            .map(|(path, mods)| Conflict { path: path.to_string(), mods })
            .collect()
    }

    /// Compare the installed build with the last one seen. On a new build,
    /// switch every mod off (safe mode). The caller then applies the change.
    pub fn check_build(&mut self, current: Option<&str>) -> Result<BuildCheck> {
        let Some(current) = current else {
            return Ok(BuildCheck::Unknown);
        };
        match self.state.last_build_id.clone() {
            None => {
                self.state.last_build_id = Some(current.to_string());
                self.save()?;
                Ok(BuildCheck::FirstSeen)
            }
            Some(prev) if prev == current => Ok(BuildCheck::Same),
            Some(prev) => {
                let disabled: Vec<String> = self.active_mods().iter().map(|m| m.name.clone()).collect();
                for m in &mut self.state.mods {
                    m.enabled = false;
                }
                let ev = SafeModeEvent {
                    from_build: prev.clone(),
                    to_build: current.to_string(),
                    at: now(),
                    disabled,
                };
                self.state.last_build_id = Some(current.to_string());
                self.state.safe_mode = Some(ev.clone());
                self.log(&format!(
                    "Deadlock updated (build {prev} -> {current}). Safe mode: switched off {} mod(s)",
                    ev.disabled.len()
                ));
                self.save()?;
                Ok(BuildCheck::Changed(ev))
            }
        }
    }

    // ----- Sound projects -----

    pub fn project(&self, id: &str) -> Result<&SoundProject> {
        self.state
            .sound_projects
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| Error::msg("That sound project no longer exists"))
    }

    fn project_mut(&mut self, id: &str) -> Result<&mut SoundProject> {
        self.state
            .sound_projects
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| Error::msg("That sound project no longer exists"))
    }

    pub fn create_project(&mut self, name: &str, hero: &str) -> Result<SoundProject> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::msg("Give the sound mod a name"));
        }
        let p = SoundProject {
            id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            name: name.to_string(),
            hero: hero.trim().to_string(),
            items: Vec::new(),
            mute_voice: Vec::new(),
            mod_id: None,
            last_built: None,
        };
        self.state.sound_projects.push(p.clone());
        self.save()?;
        Ok(p)
    }

    pub fn update_project(&mut self, id: &str, name: Option<String>, hero: Option<String>) -> Result<()> {
        let p = self.project_mut(id)?;
        if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
            p.name = n.trim().to_string();
        }
        if let Some(h) = hero {
            p.hero = h.trim().to_string();
        }
        self.save()
    }

    /// Delete a project. Its built mod stays in the library.
    pub fn delete_project(&mut self, id: &str) -> Result<()> {
        self.state.sound_projects.retain(|p| p.id != id);
        self.save()
    }

    /// Add or update a replacement. One entry per game sound.
    pub fn set_project_item(&mut self, id: &str, target: &str, source: &str) -> Result<()> {
        let target = target.to_lowercase().replace('\\', "/");
        let p = self.project_mut(id)?;
        if let Some(it) = p.items.iter_mut().find(|i| i.target == target) {
            it.source = source.to_string();
        } else {
            p.items.push(crate::csdk::SoundJob { target, source: source.to_string(), volume_db: 0.0 });
        }
        self.save()
    }

    /// Volume relative to the original, clamped to the allowed range.
    pub fn set_item_volume(&mut self, id: &str, target: &str, volume_db: f64) -> Result<()> {
        let v = volume_db.clamp(crate::audio::MIN_OFFSET_DB, crate::audio::MAX_BOOST_DB);
        let p = self.project_mut(id)?;
        let it = p
            .items
            .iter_mut()
            .find(|i| i.target == target)
            .ok_or_else(|| Error::msg("That sound isn't in this project"))?;
        it.volume_db = (v * 10.0).round() / 10.0;
        self.save()
    }

    pub fn set_voice_mute(&mut self, id: &str, hero: &str, slot: u8, on: bool) -> Result<()> {
        if !(1..=4).contains(&slot) {
            return Err(Error::msg("Ability slot must be 1 to 4"));
        }
        let p = self.project_mut(id)?;
        let m = VoiceMute { hero: hero.to_lowercase(), slot };
        p.mute_voice.retain(|x| x != &m);
        if on {
            p.mute_voice.push(m);
        }
        self.save()
    }

    pub fn remove_project_item(&mut self, id: &str, target: &str) -> Result<()> {
        self.project_mut(id)?.items.retain(|i| i.target != target);
        self.save()
    }

    /// After a build: put the VPK in the library (new mod, or a new version
    /// of the mod this project made before). Returns the mod id.
    pub fn store_build(&mut self, project_id: &str, vpk_path: &Path) -> Result<String> {
        let p = self.project(project_id)?.clone();
        let existing = p.mod_id.clone().filter(|m| self.state.mods.iter().any(|x| &x.id == m));
        let mod_id = match existing {
            Some(m) => {
                self.replace_vpk(&m, vpk_path)?;
                let entry = self.get_mut(&m)?;
                entry.name = p.name.clone();
                entry.hero = p.hero.clone();
                entry.kind = ModKind::Sound;
                let v: u32 = entry.version.parse().unwrap_or(1);
                entry.version = (v + 1).to_string();
                m
            }
            None => {
                self.import_vpk(
                    vpk_path,
                    ModMeta {
                        name: Some(p.name.clone()),
                        kind: Some(ModKind::Sound),
                        hero: Some(p.hero.clone()),
                        notes: Some("Built in Sound Builder".into()),
                        version: Some("1".into()),
                    },
                )?
                .id
            }
        };
        let pm = self.project_mut(project_id)?;
        pm.mod_id = Some(mod_id.clone());
        pm.last_built = Some(now());
        self.save()?;
        Ok(mod_id)
    }

    // ----- Model projects -----

    /// Where a model project's files live.
    pub fn model_dir(&self, id: &str) -> PathBuf {
        self.root.join("models").join(id)
    }

    pub fn model_project(&self, id: &str) -> Result<&ModelProject> {
        self.state.model_projects.iter().find(|p| p.id == id).ok_or_else(|| Error::msg("That model project no longer exists"))
    }

    fn model_project_mut(&mut self, id: &str) -> Result<&mut ModelProject> {
        self.state.model_projects.iter_mut().find(|p| p.id == id).ok_or_else(|| Error::msg("That model project no longer exists"))
    }

    pub fn create_model_project(&mut self, name: &str, hero: &str, model: &str) -> Result<ModelProject> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::msg("Give the model mod a name"));
        }
        if !model.starts_with("models/") || !model.ends_with(".vmdl") || model.contains("..") {
            return Err(Error::msg(format!("{model} isn't a hero model")));
        }
        let p = ModelProject {
            id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            name: name.to_string(),
            hero: hero.trim().to_string(),
            model: model.to_string(),
            created: now(),
            extracted: None,
            extracted_build: None,
            extracted_tool: None,
            mod_id: None,
            last_built: None,
        };
        std::fs::create_dir_all(self.model_dir(&p.id)).at(&self.root)?;
        self.state.model_projects.push(p.clone());
        self.save()?;
        Ok(p)
    }

    pub fn rename_model_project(&mut self, id: &str, name: &str) -> Result<()> {
        if name.trim().is_empty() {
            return Err(Error::msg("Give the model mod a name"));
        }
        self.model_project_mut(id)?.name = name.trim().to_string();
        self.save()
    }

    pub fn mark_extracted(&mut self, id: &str, build: Option<String>, tool: &str) -> Result<()> {
        let p = self.model_project_mut(id)?;
        p.extracted = Some(now());
        p.extracted_build = build;
        p.extracted_tool = Some(tool.to_string());
        self.save()
    }

    /// Delete a project and its working files. Its built mod stays in the
    /// library.
    pub fn delete_model_project(&mut self, id: &str) -> Result<()> {
        self.model_project(id)?;
        let dir = self.model_dir(id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).at(&dir)?;
        }
        self.state.model_projects.retain(|p| p.id != id);
        self.save()
    }

    /// Add a model build to the library (or replace the project's earlier
    /// build, bumping its version). Returns the mod id.
    pub fn store_model_build(&mut self, project_id: &str, vpk_path: &Path, notes: &str) -> Result<String> {
        let p = self.model_project(project_id)?.clone();
        let existing = p.mod_id.clone().filter(|m| self.state.mods.iter().any(|x| &x.id == m));
        let mod_id = match existing {
            Some(m) => {
                self.replace_vpk(&m, vpk_path)?;
                let entry = self.get_mut(&m)?;
                entry.name = p.name.clone();
                entry.hero = p.hero.clone();
                entry.kind = ModKind::Skin;
                entry.notes = notes.to_string();
                let v: u32 = entry.version.parse().unwrap_or(1);
                entry.version = (v + 1).to_string();
                m
            }
            None => {
                self.import_vpk(
                    vpk_path,
                    ModMeta {
                        name: Some(p.name.clone()),
                        kind: Some(ModKind::Skin),
                        hero: Some(p.hero.clone()),
                        notes: Some(notes.to_string()),
                        version: Some("1".into()),
                    },
                )?
                .id
            }
        };
        let pm = self.model_project_mut(project_id)?;
        pm.mod_id = Some(mod_id.clone());
        pm.last_built = Some(now());
        self.save()?;
        Ok(mod_id)
    }

    pub fn dismiss_safe_mode(&mut self) -> Result<()> {
        self.state.safe_mode = None;
        self.save()
    }
}

/// Best guess at a mod's type from the paths inside it.
pub fn guess_kind(files: &[String]) -> ModKind {
    let count = |prefix: &str| files.iter().filter(|f| f.starts_with(prefix)).count();
    let sounds = count("sounds/") + count("soundevents/");
    let models = count("models/") + count("materials/");
    let ui = count("panorama/") + count("resource/");
    let max = sounds.max(models).max(ui);
    if max == 0 {
        ModKind::Other
    } else if max == sounds {
        ModKind::Sound
    } else if max == models {
        ModKind::Skin
    } else {
        ModKind::Ui
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn write_vpk(dir: &Path, name: &str, paths: &[&str]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, vpk::build_test_vpk(paths)).unwrap();
        p
    }

    fn lib_with(n: usize) -> (tempfile::TempDir, Library) {
        let t = tempfile::tempdir().unwrap();
        let mut lib = Library::open(&t.path().join("data")).unwrap();
        for i in 0..n {
            let src = write_vpk(t.path(), &format!("mod_{i}.vpk"), &[&format!("sounds/s{i}.vsnd_c")]);
            lib.import_vpk(&src, ModMeta::default()).unwrap();
        }
        (t, lib)
    }

    #[test]
    fn import_copies_and_reads_files() {
        let (_t, lib) = lib_with(1);
        let m = &lib.state.mods[0];
        assert_eq!(m.name, "mod 0");
        assert_eq!(m.kind, ModKind::Sound);
        assert!(!m.enabled);
        assert_eq!(m.files, vec!["sounds/s0.vsnd_c"]);
        assert!(lib.stored_path(m).exists());
    }

    #[test]
    fn import_rejects_non_vpk() {
        let (t, mut lib) = lib_with(0);
        let p = t.path().join("fake.vpk");
        std::fs::write(&p, b"not a vpk").unwrap();
        assert!(lib.import_vpk(&p, ModMeta::default()).is_err());
        assert!(lib.state.mods.is_empty());
    }

    #[test]
    fn state_persists() {
        let (t, mut lib) = lib_with(2);
        let id = lib.state.mods[1].id.clone();
        lib.set_enabled(&id, true).unwrap();
        let again = Library::open(&t.path().join("data")).unwrap();
        assert_eq!(again.state, lib.state);
    }

    #[test]
    fn reorder_validates() {
        let (_t, mut lib) = lib_with(3);
        let ids: Vec<String> = lib.state.mods.iter().rev().map(|m| m.id.clone()).collect();
        lib.reorder(&ids).unwrap();
        assert_eq!(lib.state.mods[0].id, ids[0]);
        let bad = vec![ids[0].clone(), ids[0].clone(), ids[1].clone()];
        assert!(lib.reorder(&bad).is_err());
        assert_eq!(lib.state.mods.len(), 3);
    }

    #[test]
    fn profiles_round_trip() {
        let (_t, mut lib) = lib_with(3);
        let a = lib.state.mods[0].id.clone();
        let c = lib.state.mods[2].id.clone();
        lib.set_enabled(&a, true).unwrap();
        lib.set_enabled(&c, true).unwrap();
        let p = lib.save_profile("Sounds only").unwrap();
        assert_eq!(p.mods, vec![a.clone(), c.clone()]);
        lib.set_all_enabled(false).unwrap();
        // Reverse the order inside the profile, then apply it.
        lib.state.profiles[0].mods = vec![c.clone(), a.clone()];
        lib.apply_profile(&p.id).unwrap();
        let ids: Vec<&str> = lib.state.mods.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(&ids[..2], &[c.as_str(), a.as_str()]);
        assert!(lib.state.mods[0].enabled && lib.state.mods[1].enabled && !lib.state.mods[2].enabled);
        // Same name overwrites
        lib.save_profile("sounds ONLY").unwrap();
        assert_eq!(lib.state.profiles.len(), 1);
    }

    #[test]
    fn conflicts_found_between_enabled_mods() {
        let (t, mut lib) = lib_with(0);
        let a = write_vpk(t.path(), "a.vpk", &["sounds/x.vsnd_c", "sounds/a.vsnd_c"]);
        let b = write_vpk(t.path(), "b.vpk", &["sounds/x.vsnd_c"]);
        let ma = lib.import_vpk(&a, ModMeta::default()).unwrap();
        let mb = lib.import_vpk(&b, ModMeta::default()).unwrap();
        assert!(lib.conflicts().is_empty());
        lib.set_enabled(&ma.id, true).unwrap();
        lib.set_enabled(&mb.id, true).unwrap();
        let c = lib.conflicts();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].path, "sounds/x.vsnd_c");
        assert_eq!(c[0].mods, vec![ma.id, mb.id]);
    }

    #[test]
    fn safe_mode_disables_everything_on_new_build() {
        let (_t, mut lib) = lib_with(2);
        lib.set_all_enabled(true).unwrap();
        assert_eq!(lib.check_build(None).unwrap(), BuildCheck::Unknown);
        assert_eq!(lib.check_build(Some("100")).unwrap(), BuildCheck::FirstSeen);
        assert_eq!(lib.check_build(Some("100")).unwrap(), BuildCheck::Same);
        assert!(lib.state.mods.iter().all(|m| m.enabled));
        match lib.check_build(Some("101")).unwrap() {
            BuildCheck::Changed(ev) => {
                assert_eq!(ev.from_build, "100");
                assert_eq!(ev.disabled.len(), 2);
            }
            other => panic!("expected Changed, got {other:?}"),
        }
        assert!(lib.state.mods.iter().all(|m| !m.enabled));
        assert!(lib.state.safe_mode.is_some());
    }

    #[test]
    fn replace_clears_verified() {
        let (t, mut lib) = lib_with(1);
        let id = lib.state.mods[0].id.clone();
        lib.mark_verified(&id, Some("100".into())).unwrap();
        let stamp = lib.state.mods[0].stamp.clone();
        let src = write_vpk(t.path(), "v2.vpk", &["sounds/new.vsnd_c", "sounds/two.vsnd_c"]);
        lib.replace_vpk(&id, &src).unwrap();
        let m = &lib.state.mods[0];
        assert_eq!(m.verified_build, None);
        assert_ne!(m.stamp, stamp);
        assert_eq!(m.files.len(), 2);
    }

    #[test]
    fn sound_project_builds_into_one_library_mod() {
        let (t, mut lib) = lib_with(0);
        let p = lib.create_project("Storm remix", "Seven").unwrap();
        lib.set_project_item(&p.id, "Sounds/A/B_lp.vsnd_c", "C:/x.wav").unwrap();
        lib.set_project_item(&p.id, "sounds/a/b_lp.vsnd_c", "C:/y.wav").unwrap();
        assert_eq!(lib.project(&p.id).unwrap().items.len(), 1, "same target updates in place");
        assert_eq!(lib.project(&p.id).unwrap().items[0].source, "C:/y.wav");
        lib.set_item_volume(&p.id, "sounds/a/b_lp.vsnd_c", 9.0).unwrap();
        assert_eq!(lib.project(&p.id).unwrap().items[0].volume_db, crate::audio::MAX_BOOST_DB, "capped");
        lib.set_voice_mute(&p.id, "GIGAWATT", 4, true).unwrap();
        lib.set_voice_mute(&p.id, "gigawatt", 4, true).unwrap();
        assert_eq!(lib.project(&p.id).unwrap().mute_voice.len(), 1);
        assert!(lib.set_voice_mute(&p.id, "gigawatt", 5, true).is_err());

        let v1 = write_vpk(t.path(), "b1.vpk", &["sounds/a/b_lp.vsnd_c"]);
        let id = lib.store_build(&p.id, &v1).unwrap();
        let m = lib.get(&id).unwrap().clone();
        assert_eq!((m.name.as_str(), m.hero.as_str(), m.kind, m.version.as_str()), ("Storm remix", "Seven", ModKind::Sound, "1"));
        lib.set_enabled(&id, true).unwrap();
        lib.mark_verified(&id, Some("1".into())).unwrap();

        let v2 = write_vpk(t.path(), "b2.vpk", &["sounds/a/b_lp.vsnd_c", "sounds/a/c.vsnd_c"]);
        let id2 = lib.store_build(&p.id, &v2).unwrap();
        assert_eq!(id, id2, "rebuild updates the same mod");
        let m2 = lib.get(&id).unwrap();
        assert_eq!(m2.version, "2");
        assert!(m2.enabled, "stays on");
        assert_eq!(m2.verified_build, None, "new build needs testing again");
        assert_eq!(m2.files.len(), 2);
        assert_eq!(lib.state.mods.len(), 1);

        // If the mod was deleted, the next build makes a new one.
        lib.remove_mod(&id).unwrap();
        let id3 = lib.store_build(&p.id, &v2).unwrap();
        assert_ne!(id3, id);
    }

    #[test]
    fn remove_deletes_file_and_profile_refs() {
        let (_t, mut lib) = lib_with(2);
        let id = lib.state.mods[0].id.clone();
        lib.set_enabled(&id, true).unwrap();
        lib.save_profile("p").unwrap();
        let path = lib.stored_path(&lib.state.mods[0]);
        lib.remove_mod(&id).unwrap();
        assert!(!path.exists());
        assert_eq!(lib.state.mods.len(), 1);
        assert!(lib.state.profiles[0].mods.is_empty());
    }

    #[test]
    fn model_projects_build_into_a_skin_mod() {
        let t = tempfile::tempdir().unwrap();
        let mut lib = Library::open(t.path()).unwrap();
        assert!(lib.create_model_project(" ", "gigawatt", "models/a/b.vmdl").is_err());
        assert!(lib.create_model_project("x", "gigawatt", "sounds/a.vsnd").is_err());
        let p = lib.create_model_project("Seven test", "gigawatt", "models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner.vmdl").unwrap();
        assert!(lib.model_dir(&p.id).is_dir());
        lib.mark_extracted(&p.id, Some("123".into()), "abc").unwrap();
        assert_eq!(lib.model_project(&p.id).unwrap().extracted_tool.as_deref(), Some("abc"));
        assert_eq!(lib.model_project(&p.id).unwrap().extracted_build.as_deref(), Some("123"));

        let v1 = write_vpk(t.path(), "m1.vpk", &["models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner.vmdl_c"]);
        let id = lib.store_model_build(&p.id, &v1, "Built in Model Builder").unwrap();
        let m = lib.get(&id).unwrap();
        assert_eq!(m.kind, ModKind::Skin);
        assert_eq!(m.hero, "gigawatt");
        let id2 = lib.store_model_build(&p.id, &v1, "again").unwrap();
        assert_eq!(id, id2);
        assert_eq!(lib.get(&id).unwrap().version, "2");

        // Reopen: saved. Delete: files gone, mod stays.
        let mut lib = Library::open(t.path()).unwrap();
        assert_eq!(lib.state.model_projects.len(), 1);
        lib.delete_model_project(&p.id).unwrap();
        assert!(!lib.model_dir(&p.id).exists());
        assert!(lib.get(&id).is_ok());
    }

    #[test]
    fn packs_group_switch_and_load_together() {
        let (_t, mut lib) = lib_with(5);
        let ids: Vec<String> = lib.state.mods.iter().map(|m| m.id.clone()).collect();
        for id in &ids {
            lib.set_enabled(id, true).unwrap();
        }
        // Pack mods 1 and 3: they end up next to each other at mod 1's place.
        let p = lib.create_pack("Seven", &[ids[3].clone(), ids[1].clone()]).unwrap();
        let order: Vec<&str> = lib.state.mods.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(order, [&ids[0], &ids[1], &ids[3], &ids[2], &ids[4]].iter().map(|s| s.as_str()).collect::<Vec<_>>());
        assert_eq!(p.mods, vec![ids[1].clone(), ids[3].clone()]);
        // The pack switch overrides the members' own switches.
        lib.set_pack_enabled(&p.id, false).unwrap();
        let active: Vec<&str> = lib.active_mods().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(active, vec![ids[0].as_str(), ids[2].as_str(), ids[4].as_str()]);
        assert!(lib.get(&ids[1]).unwrap().enabled, "member keeps its own switch");
        lib.set_pack_enabled(&p.id, true).unwrap();
        assert_eq!(lib.active_mods().len(), 5);
        // Reordering can't split a pack.
        let mut r: Vec<String> = lib.state.mods.iter().map(|m| m.id.clone()).collect();
        let x = r.remove(2); // ids[3]
        r.push(x);
        lib.reorder(&r).unwrap();
        let pos = |lib: &Library, id: &str| lib.state.mods.iter().position(|m| m.id == id).unwrap();
        assert_eq!(pos(&lib, &ids[3]), pos(&lib, &ids[1]) + 1);
        // Joining and leaving.
        lib.set_mod_pack(&ids[4], Some(&p.id)).unwrap();
        assert_eq!(lib.pack_of(&ids[4]).unwrap().mods.last().unwrap(), &ids[4]);
        assert_eq!(pos(&lib, &ids[4]), pos(&lib, &ids[3]) + 1);
        lib.set_mod_pack(&ids[4], None).unwrap();
        assert!(lib.pack_of(&ids[4]).is_none());
        // Removing every member removes the pack.
        lib.remove_mod(&ids[1]).unwrap();
        lib.remove_mod(&ids[3]).unwrap();
        assert!(lib.state.packs.is_empty());
    }

    #[test]
    fn packs_export_and_import() {
        let (t, mut lib) = lib_with(3);
        let ids: Vec<String> = lib.state.mods.iter().map(|m| m.id.clone()).collect();
        lib.set_enabled(&ids[0], true).unwrap();
        lib.update_meta(&ids[0], ModMeta { hero: Some("Seven".into()), ..Default::default() }).unwrap();
        let p = lib.create_pack("Full Seven", &[ids[0].clone(), ids[2].clone()]).unwrap();
        lib.update_pack(&p.id, None, Some("hat and sounds".into())).unwrap();
        let file = t.path().join(format!("seven.{PACK_EXT}"));
        lib.export_pack(&p.id, &file).unwrap();
        let before = lib.state.mods.len();
        let q = lib.import_pack(&file).unwrap();
        assert_eq!(lib.state.mods.len(), before + 2);
        assert_eq!(q.name, "Full Seven");
        assert_eq!(q.notes, "hat and sounds");
        assert!(!lib.state.packs.iter().find(|x| x.id == q.id).unwrap().enabled, "imports arrive off");
        let first = lib.get(&q.mods[0]).unwrap();
        assert_eq!(first.hero, "Seven");
        assert!(first.enabled, "members keep their own switches");
        assert_eq!(first.files, lib.get(&ids[0]).unwrap().files);
        assert!(lib.active_mods().iter().all(|m| !q.mods.contains(&m.id)));
        // Delete the pack but keep its mods; then delete one with its mods.
        lib.delete_pack(&p.id, false).unwrap();
        assert_eq!(lib.state.mods.len(), before + 2);
        lib.delete_pack(&q.id, true).unwrap();
        assert_eq!(lib.state.mods.len(), before);
        assert!(lib.import_pack(&lib.stored_path(lib.get(&ids[1]).unwrap())).is_err(), "a VPK isn't a pack");
    }
}
