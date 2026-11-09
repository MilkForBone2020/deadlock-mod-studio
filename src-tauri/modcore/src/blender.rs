//! Blender: finding it, installing the companion add-on, and opening a
//! model project in it.
//!
//! The add-on's source lives in `blender/deadlock_mod_studio/` in this repo
//! and is built into the app. It's installed with Blender's own extension
//! command (`blender --command extension install-file`), so the app never
//! writes into Blender's folders itself.

use crate::{steam, Error, IoCtx, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const ADDON_ID: &str = "deadlock_mod_studio";
pub const ADDON_INIT: &str = include_str!("../../../blender/deadlock_mod_studio/__init__.py");
pub const ADDON_MANIFEST: &str = include_str!("../../../blender/deadlock_mod_studio/blender_manifest.toml");
pub const ADDON_FIT: &str = include_str!("../../../blender/deadlock_mod_studio/fit.py");

/// The add-on's version, from its manifest.
pub fn addon_version() -> String {
    ADDON_MANIFEST
        .lines()
        .find_map(|l| l.trim().strip_prefix("version").and_then(|r| r.trim().strip_prefix('=')).map(|v| v.trim().trim_matches('"').to_string()))
        .unwrap_or_default()
}

/// The add-on packed as a Blender extension zip.
pub fn addon_zip() -> Result<Vec<u8>> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, text) in [("blender_manifest.toml", ADDON_MANIFEST), ("__init__.py", ADDON_INIT), ("fit.py", ADDON_FIT)] {
            z.start_file(name, opts).map_err(|e| Error::msg(format!("zip: {e}")))?;
            z.write_all(text.as_bytes()).map_err(|e| Error::msg(format!("zip: {e}")))?;
        }
        z.finish().map_err(|e| Error::msg(format!("zip: {e}")))?;
    }
    Ok(buf.into_inner())
}

/// Version from a folder like `Blender 5.0` or `blender-4.2.3-windows-x64`.
fn version_of(path: &Path) -> (u32, u32) {
    for comp in path.components().rev() {
        let s = comp.as_os_str().to_string_lossy().to_lowercase();
        let digits: String = s.trim_start_matches(|c: char| !c.is_ascii_digit()).chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        let mut it = digits.split('.').filter_map(|x| x.parse::<u32>().ok());
        if let (Some(a), Some(b)) = (it.next(), it.next()) {
            return (a, b);
        }
    }
    (0, 0)
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BlenderInstall {
    pub exe: String,
    pub version: String,
}

/// Every Blender found in the usual places, newest first.
pub fn find_installs(extra_roots: &[PathBuf]) -> Vec<BlenderInstall> {
    let mut found: Vec<PathBuf> = vec![];
    let mut roots: Vec<PathBuf> = extra_roots.to_vec();
    for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(p) = std::env::var_os(var) {
            roots.push(PathBuf::from(p).join("Blender Foundation"));
        }
    }
    roots.push(PathBuf::from("C:\\Program Files\\Blender Foundation"));
    for root in &roots {
        if let Ok(rd) = std::fs::read_dir(root) {
            for e in rd.flatten() {
                let exe = e.path().join("blender.exe");
                if exe.is_file() {
                    found.push(exe);
                }
            }
        }
        let exe = root.join("blender.exe");
        if exe.is_file() {
            found.push(exe);
        }
    }
    // Steam
    let mut libs = vec![];
    for steam_dir in ["C:\\Program Files (x86)\\Steam", "C:\\Program Files\\Steam"] {
        let vdf = Path::new(steam_dir).join("steamapps").join("libraryfolders.vdf");
        if let Ok(t) = std::fs::read_to_string(&vdf) {
            libs.extend(steam::library_paths(&t));
        }
        libs.push(PathBuf::from(steam_dir));
    }
    for lib in libs {
        let exe = lib.join("steamapps").join("common").join("Blender").join("blender.exe");
        if exe.is_file() {
            found.push(exe);
        }
    }
    found.sort();
    found.dedup();
    let mut out: Vec<BlenderInstall> = found
        .into_iter()
        .map(|exe| {
            let (a, b) = version_of(&exe);
            BlenderInstall { version: if a > 0 { format!("{a}.{b}") } else { String::new() }, exe: exe.display().to_string() }
        })
        .collect();
    out.sort_by_key(|i| std::cmp::Reverse(version_of(Path::new(&i.exe))));
    out
}

/// Is this a Blender the add-on supports (4.2 or newer)?
pub fn supported(exe: &Path) -> bool {
    let v = version_of(exe);
    v == (0, 0) || v >= (4, 2)
}

/// Arguments to install and enable the add-on from `zip`.
pub fn install_args(zip: &Path) -> Vec<String> {
    vec![
        "--command".into(),
        "extension".into(),
        "install-file".into(),
        "--repo".into(),
        "user_default".into(),
        "--enable".into(),
        zip.display().to_string(),
    ]
}

/// Arguments to open a project: its work file when it has one, else a
/// fresh Blender that imports the hero.
pub fn open_args(project_file: &Path, work_blend: &Path) -> Vec<String> {
    if work_blend.is_file() {
        return vec![work_blend.display().to_string()];
    }
    let path = serde_json::to_string(&project_file.display().to_string()).unwrap_or_default();
    vec![
        "--python-expr".into(),
        format!("import bpy; bpy.ops.dms.open_project(filepath={path})"),
    ]
}

/// Write the add-on zip into `tools` and return its path.
pub fn write_addon_zip(tools: &Path) -> Result<PathBuf> {
    std::fs::create_dir_all(tools).at(tools)?;
    let p = tools.join(format!("{ADDON_ID}-{}.zip", addon_version()));
    std::fs::write(&p, addon_zip()?).at(&p)?;
    Ok(p)
}

/// The edited models sent back from Blender, newest first.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReturnedObject {
    pub object: String,
    pub part: String,
    #[serde(default)]
    pub triangles: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Returned {
    #[serde(default)]
    pub folder: String,
    pub sent: String,
    #[serde(default)]
    pub blender: String,
    #[serde(default)]
    pub addon: String,
    pub glb: String,
    pub objects: Vec<ReturnedObject>,
    #[serde(default)]
    pub materials: Vec<String>,
    /// Every material the model uses, with repainted textures and values.
    #[serde(default)]
    pub material_files: Vec<MaterialFile>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct MaterialFile {
    pub name: String,
    /// Path (inside the returned folder) of a repainted colour texture.
    #[serde(default)]
    pub color: Option<String>,
    /// Not one of the hero's materials (as the add-on saw it; the app
    /// decides again by name).
    #[serde(default)]
    pub new: bool,
    /// Base Color (linear RGBA) when no image is plugged in.
    #[serde(default)]
    pub color_value: Option<[f32; 4]>,
    /// Principled Roughness / Metallic values, when not driven by a texture.
    #[serde(default)]
    pub roughness: Option<f32>,
    #[serde(default)]
    pub metallic: Option<f32>,
}

pub fn returned(return_dir: &Path) -> Vec<Returned> {
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(return_dir) {
        for e in rd.flatten() {
            let m = e.path().join("manifest.json");
            if let Ok(bytes) = std::fs::read(&m) {
                if let Ok(mut r) = serde_json::from_slice::<Returned>(&bytes) {
                    r.folder = e.path().display().to_string();
                    if e.path().join(&r.glb).is_file() {
                        out.push(r);
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| b.sent.cmp(&a.sent));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addon_packs_with_its_version() {
        assert_eq!(addon_version(), "0.1.10");
        let z = addon_zip().unwrap();
        let mut a = zip::ZipArchive::new(std::io::Cursor::new(z)).unwrap();
        let names: Vec<String> = (0..a.len()).map(|i| a.by_index(i).unwrap().name().to_string()).collect();
        assert_eq!(names, vec!["blender_manifest.toml", "__init__.py", "fit.py"]);
        assert!(ADDON_INIT.contains("ADDON_VERSION = \"0.1.10\""), "add-on and manifest versions match");
    }

    #[test]
    fn versions_and_args() {
        assert_eq!(version_of(Path::new("C:\\Program Files\\Blender Foundation\\Blender 5.0\\blender.exe")), (5, 0));
        assert_eq!(version_of(Path::new("D:/x/blender-4.2.3-windows-x64/blender.exe")), (4, 2));
        assert!(supported(Path::new("C:/Blender 5.0/blender.exe")));
        assert!(!supported(Path::new("C:/Blender 3.6/blender.exe")));
        assert!(supported(Path::new("D:/SteamLibrary/steamapps/common/Blender/blender.exe")), "unknown version: try it");
        let t = tempfile::tempdir().unwrap();
        let pj = t.path().join("p q").join("dms_project.json");
        let a = open_args(&pj, &t.path().join("work.blend"));
        assert_eq!(a[0], "--python-expr");
        assert!(a[1].contains("dms.open_project(filepath=\""), "{}", a[1]);
        std::fs::write(t.path().join("work.blend"), b"x").unwrap();
        assert_eq!(open_args(&pj, &t.path().join("work.blend")).len(), 1);
        assert_eq!(install_args(Path::new("a.zip"))[..3], ["--command", "extension", "install-file"]);
    }

    #[test]
    fn reads_returned_models() {
        let t = tempfile::tempdir().unwrap();
        for (stamp, sent) in [("a", "2026-11-08T10:00:00"), ("b", "2026-11-08T11:00:00")] {
            let d = t.path().join(stamp);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("model.glb"), b"x").unwrap();
            std::fs::write(
                d.join("manifest.json"),
                format!(r#"{{"format":1,"sent":"{sent}","blender":"5.0.0","addon":"0.1.0","glb":"model.glb","objects":[{{"object":"gun","part":"gun","triangles":10}}]}}"#),
            )
            .unwrap();
        }
        std::fs::create_dir_all(t.path().join("broken")).unwrap();
        let r = returned(t.path());
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].sent, "2026-11-08T11:00:00");
        assert_eq!(r[0].objects[0].part, "gun");
    }
}
