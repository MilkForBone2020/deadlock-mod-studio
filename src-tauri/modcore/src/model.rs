//! Hero model mods: find a hero's model, take it apart with Source 2
//! Viewer's command-line tool, and rebuild it with CSDK's resource compiler.
//!
//! Extract writes, per project:
//!   original/            the decompiled ModelDoc source (`.vmdl` + mesh DMX)
//!                        and materials, laid out like the game's files
//!   original/<model>_c   the compiled original, kept as the animation source
//!   blender/<name>.glb   meshes, skeleton and materials for Blender
//!   return/              where edited meshes come back (later phases)
//!
//! Build copies the template into a CSDK addon, points its animations at a
//! copy of the original (`<name>_backup.vmdl`, the community method), puts
//! the files the compiler needs to resolve next to it, compiles, and packs
//! only what the build produced plus the backup.

use crate::kv3::{self, Kv as BKv};
use crate::kv3text::{self, Kv};
use crate::{csdk, steam, vpk, Error, IoCtx, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Source 2 Viewer command-line release the app downloads and uses.
pub const S2V_VERSION: &str = "20.1";
pub const S2V_URL: &str =
    "https://github.com/ValveResourceFormat/ValveResourceFormat/releases/download/20.1/cli-windows-x64.zip";
pub const S2V_SHA256: &str = "67aaf8a67a85ca60fd09ff343e62103a4cdf3a2d177fc656231d7d734c5b3aa3";
pub const S2V_EXE: &str = "Source2Viewer-CLI.exe";

/// A few animations to include in the Blender file, so the skeleton comes
/// with it and poses can be tested. Names a hero doesn't have are skipped.
pub const BLENDER_ANIMS: &str = "primary_stand_idle,primary_run_n,primary_crouch_idle,ui_hero_pose";

/// A hero and where its model lives, from `scripts/heroes.vdata`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HeroModel {
    pub code: String,
    pub model: String,
    /// Hero icons (`file://{images}/heroes/x_card.psd` ...), by field name.
    pub icons: Vec<(String, String)>,
}

pub fn hero_models(heroes_vdata: &BKv) -> Vec<HeroModel> {
    let mut out = vec![];
    for (key, h) in heroes_vdata.as_object().unwrap_or(&[]) {
        let Some(code) = key.strip_prefix("hero_") else { continue };
        let Some(model) = h.get("m_strModelName").and_then(|v| v.as_str()) else { continue };
        if model.is_empty() || !model.ends_with(".vmdl") {
            continue;
        }
        let icons = ["m_strIconHeroCard", "m_strIconHeroCardCritical", "m_strIconHeroCardGloat", "m_strIconImageSmall", "m_strMinimapImage", "m_strTopBarVertical"]
            .iter()
            .filter_map(|f| h.get(f).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| (f.to_string(), s.to_string())))
            .collect();
        out.push(HeroModel { code: code.to_string(), model: model.to_string(), icons });
    }
    out
}

/// Paths a compiled resource refers to (its RERL block).
pub fn external_refs(res: &[u8]) -> Result<Vec<String>> {
    let block = match kv3::resource_block(res, b"RERL") {
        Ok(b) => b,
        Err(_) => return Ok(vec![]),
    };
    let u32at = |o: usize| -> Result<usize> {
        block.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize).ok_or_else(|| Error::msg("bad RERL block"))
    };
    let (rel, count) = (u32at(0)?, u32at(4)?);
    let mut out = vec![];
    for i in 0..count {
        let e = rel + i * 16;
        let name_at = e + 8 + u32at(e + 8)?;
        let tail = block.get(name_at..).ok_or_else(|| Error::msg("bad RERL name"))?;
        let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
        out.push(String::from_utf8_lossy(&tail[..end]).into_owned());
    }
    Ok(out)
}

/// What a compiled hero model contains, for the app to show and check.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelSummary {
    pub bones: Vec<String>,
    pub meshes: usize,
    pub lods: usize,
    pub body_groups: Vec<String>,
    pub materials: Vec<String>,
    pub attachments: Vec<String>,
    pub hitboxes: usize,
    pub ragdoll_bodies: usize,
    pub cloth_nodes: usize,
    pub anim_graphs: Vec<String>,
    pub triangles_lod0: u64,
}

fn num(v: &BKv) -> u64 {
    match v {
        BKv::Int(i) => *i as u64,
        BKv::UInt(u) => *u,
        BKv::Float(f) => *f as u64,
        _ => 0,
    }
}

/// Every block of one type, in order.
fn blocks<'a>(res: &'a [u8], kind: &[u8; 4]) -> Vec<&'a [u8]> {
    let u = |o: usize| res.get(o..o + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize).unwrap_or(0);
    let (bo, n) = (u(8), u(12));
    let mut out = vec![];
    for i in 0..n {
        let o = 8 + bo + i * 12;
        if res.get(o..o + 4) == Some(&kind[..]) {
            let st = o + 4 + u(o + 4);
            if let Some(b) = res.get(st..st + u(o + 8)) {
                out.push(b);
            }
        }
    }
    out
}

pub fn summarise(res: &[u8]) -> Result<ModelSummary> {
    let data = kv3::parse_resource(res)?;
    let mut s = ModelSummary::default();
    if let Some(sk) = data.get("m_modelSkeleton") {
        s.bones = sk.get("m_boneName").and_then(|v| v.as_array()).unwrap_or(&[]).iter().filter_map(|b| b.as_str().map(String::from)).collect();
    }
    s.lods = data.get("m_lodGroupSwitchDistances").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    // "DEFAULT_@0_#&BASE" -> "DEFAULT: BASE"
    s.body_groups = data
        .get("m_meshGroups")
        .and_then(|v| v.as_array())
        .unwrap_or(&[])
        .iter()
        .filter_map(|g| g.as_str())
        .map(|g| match g.split_once("_@") {
            Some((group, rest)) => format!("{group}: {}", rest.split("#&").nth(1).unwrap_or(rest)),
            None => g.to_string(),
        })
        .collect();
    s.anim_graphs = data.get("m_animGraph2Refs").and_then(|v| v.as_array()).unwrap_or(&[]).iter().filter_map(|g| g.get("m_hGraph").and_then(|v| v.as_str()).map(String::from)).collect();
    let lod_masks: Vec<u64> = data.get("m_refLODGroupMasks").and_then(|v| v.as_array()).unwrap_or(&[]).iter().map(num).collect();
    s.materials = external_refs(res)?.into_iter().filter(|r| r.ends_with(".vmat")).collect();

    for (i, block) in blocks(res, b"MDAT").into_iter().enumerate() {
        let Ok(mesh) = kv3::parse(block) else { continue };
        s.meshes += 1;
        if s.attachments.is_empty() {
            s.attachments = mesh
                .get("m_attachments")
                .and_then(|v| v.as_array())
                .unwrap_or(&[])
                .iter()
                .filter_map(|a| a.get("value").unwrap_or(a).get("m_name").and_then(|n| n.as_str()).map(String::from))
                .collect();
        }
        if s.hitboxes == 0 {
            for hs in mesh.get("m_hitboxsets").and_then(|v| v.as_array()).unwrap_or(&[]) {
                s.hitboxes += hs.get("value").unwrap_or(hs).get("m_HitBoxes").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
            }
        }
        if lod_masks.get(i).copied().unwrap_or(1) & 1 == 1 {
            for so in mesh.get("m_sceneObjects").and_then(|v| v.as_array()).unwrap_or(&[]) {
                for dc in so.get("m_drawCalls").and_then(|v| v.as_array()).unwrap_or(&[]) {
                    s.triangles_lod0 += dc.get("m_nIndexCount").map(num).unwrap_or(0) / 3;
                }
            }
        }
    }
    if let Some(phys) = blocks(res, b"PHYS").first().and_then(|b| kv3::parse(b).ok()) {
        s.ragdoll_bodies = phys.get("m_parts").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        s.cloth_nodes = phys.get("m_pFeModel").and_then(|f| f.get("m_CtrlName")).and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
    }
    Ok(s)
}

/// Runs a tool with arguments. Returns (success, output).
pub type ToolRunner<'a> = dyn Fn(&Path, &[String]) -> Result<(bool, String)> + 'a;

/// The real tool runner, hidden window on Windows.
pub fn run_tool(exe: &Path, args: &[String]) -> Result<(bool, String)> {
    let mut cmd = std::process::Command::new(exe);
    if let Some(d) = exe.parent() {
        cmd.current_dir(d);
    }
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().at(exe)?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), text))
}

/// `models/a/b.vmdl` -> ("models/a", "b")
fn split_model(model: &str) -> Result<(String, String)> {
    let m = model.replace('\\', "/").to_lowercase();
    let stem = m.strip_suffix(".vmdl").ok_or_else(|| Error::msg(format!("{model} isn't a model (.vmdl)")))?;
    if !stem.starts_with("models/") || stem.contains("..") {
        return Err(Error::msg(format!("{model} isn't under models/")));
    }
    let (dir, name) = stem.rsplit_once('/').unwrap();
    Ok((dir.to_string(), name.to_string()))
}

pub fn backup_path(model: &str) -> Result<String> {
    let (dir, name) = split_model(model)?;
    Ok(format!("{dir}/{name}_backup.vmdl"))
}

fn native(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(root.to_path_buf(), |p, s| p.join(s))
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ExtractReport {
    pub summary: ModelSummary,
    pub blender_file: String,
    pub template: String,
    pub mesh_files: Vec<String>,
    pub log: String,
}

/// Take a hero's model apart into `project_dir`.
pub fn extract(
    s2v: &Path,
    game_dir: &Path,
    model: &str,
    project_dir: &Path,
    runner: &ToolRunner,
    progress: &dyn Fn(&str),
) -> Result<ExtractReport> {
    let (dir, name) = split_model(model)?;
    let pak = steam::citadel_dir(game_dir).join("pak01_dir.vpk");
    progress("Reading the model from the game");
    let index = vpk::read_directory(&pak)?;
    let entry = index
        .find(&format!("{dir}/{name}.vmdl_c"))
        .ok_or_else(|| Error::msg(format!("{model} isn't in the game's files (was it renamed in an update?)")))?;
    let compiled = index.extract(entry)?;
    let summary = summarise(&compiled)?;

    let original = project_dir.join("original");
    if original.exists() {
        std::fs::remove_dir_all(&original).at(&original)?;
    }
    std::fs::create_dir_all(&original).at(&original)?;
    for d in ["blender", "return"] {
        std::fs::create_dir_all(project_dir.join(d)).at(project_dir)?;
    }
    let kept = native(&original, &format!("{dir}/{name}.vmdl_c"));
    std::fs::create_dir_all(kept.parent().unwrap()).at(&kept)?;
    std::fs::write(&kept, &compiled).at(&kept)?;

    // Decompile the model and its materials. Two or more filters make the
    // tool write a folder tree instead of a single file.
    progress("Decompiling the model (about half a minute)");
    let mut filters = vec![format!("{dir}/{name}.vmdl_c")];
    filters.extend(summary.materials.iter().map(|m| format!("{m}_c")));
    if filters.len() < 2 {
        filters.push(format!("{dir}/{name}.vnmskel_c"));
    }
    let args = vec![
        "-i".into(),
        pak.display().to_string(),
        "-f".into(),
        filters.join(","),
        "-d".into(),
        "-o".into(),
        original.display().to_string(),
    ];
    let (_, out1) = runner(s2v, &args)?;
    let template = native(&original, &format!("{dir}/{name}.vmdl"));
    if !template.is_file() {
        return Err(Error::msg(format!("Source 2 Viewer didn't write the model source.\n\n{}", tail(&out1))));
    }

    progress("Exporting for Blender");
    let glb = project_dir.join("blender").join(format!("{name}.glb"));
    let args = vec![
        "-i".into(),
        pak.display().to_string(),
        "-f".into(),
        format!("{dir}/{name}.vmdl_c"),
        "-o".into(),
        glb.display().to_string(),
        "-d".into(),
        "--gltf_export_format".into(),
        "glb".into(),
        "--gltf_export_materials".into(),
        "--gltf_export_animations".into(),
        "--gltf_animation_list".into(),
        BLENDER_ANIMS.into(),
    ];
    let (_, out2) = runner(s2v, &args)?;
    if !glb.is_file() {
        return Err(Error::msg(format!("Source 2 Viewer didn't write the Blender file.\n\n{}", tail(&out2))));
    }

    // Animations come from the backup at build time, so the decompiled
    // animation files aren't needed: drop them to save ~200 MB.
    progress("Tidying up");
    let text = std::fs::read_to_string(&template).at(&template)?;
    let doc = kv3text::parse(&text)?;
    let mesh_files = mesh_files(&doc);
    let keep: BTreeSet<String> = mesh_files.iter().map(|f| f.to_lowercase()).collect();
    let model_folder = native(&original, &dir);
    for e in std::fs::read_dir(&model_folder).at(&model_folder)?.flatten() {
        let p = e.path();
        let fname = e.file_name().to_string_lossy().to_lowercase();
        let rel = format!("{dir}/{fname}");
        if p.is_dir() && fname == "clips" {
            let _ = std::fs::remove_dir_all(&p);
        } else if fname.ends_with(".dmx") && !keep.contains(&rel) {
            let _ = std::fs::remove_file(&p);
        }
    }
    std::fs::write(project_dir.join("summary.json"), serde_json::to_vec_pretty(&summary)?).at(project_dir)?;

    Ok(ExtractReport {
        summary,
        blender_file: glb.display().to_string(),
        template: template.display().to_string(),
        mesh_files,
        log: format!("{}\n{}", tail(&out1), tail(&out2)),
    })
}

fn tail(s: &str) -> String {
    let lines: Vec<&str> = s.lines().filter(|l| !l.contains("Dump written")).collect();
    lines[lines.len().saturating_sub(40)..].join("\n")
}

fn root_children(doc: &Kv) -> Option<&Vec<Kv>> {
    doc.get("rootNode").and_then(|r| r.children())
}

/// Mesh and physics files the model source uses (outside its animations).
pub fn mesh_files(doc: &kv3text::Doc) -> Vec<String> {
    let mut out = BTreeSet::new();
    for list in root_children(&doc.root).map(|v| v.as_slice()).unwrap_or(&[]) {
        if list.class().as_deref() == Some("AnimationList") {
            continue;
        }
        let mut all = vec![];
        list.walk(&mut all);
        for n in all {
            if let Some(f) = n.get("filename").and_then(|v| v.as_str()) {
                let f = f.replace('\\', "/");
                let l = f.to_lowercase();
                if [".dmx", ".fbx", ".obj", ".smd"].iter().any(|e| l.ends_with(e)) {
                    out.insert(f);
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Point the model's animations at the backup copy of the original
/// instead of recompiling every animation.
pub fn use_backup_animations(doc: &mut kv3text::Doc, model: &str) -> Result<()> {
    let backup = backup_path(model)?;
    let include = Kv::Object(vec![("_class".into(), Kv::str("AnimIncludeModel")), ("model".into(), Kv::str(&backup))]);
    let root = doc.root.get_mut("rootNode").ok_or_else(|| Error::msg("model source has no rootNode"))?;
    let kids = root.children_mut().ok_or_else(|| Error::msg("model source has no nodes"))?;
    match kids.iter_mut().find(|k| k.class().as_deref() == Some("AnimationList")) {
        Some(list) => list.set("children", Kv::Array(vec![include])),
        None => kids.push(Kv::Object(vec![
            ("_class".into(), Kv::str("AnimationList")),
            ("children".into(), Kv::Array(vec![include])),
        ])),
    }
    Ok(())
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ModelBuildReport {
    pub vpk: String,
    pub vpk_size: u64,
    pub files: Vec<String>,
    pub warnings: Vec<String>,
    pub log: String,
}

/// Build a model mod from an extracted project. `slug` names the CSDK addon.
#[allow(clippy::too_many_arguments)]
pub fn build(
    csdk_dir: &Path,
    game_dir: &Path,
    model: &str,
    project_dir: &Path,
    slug: &str,
    out_vpk: &Path,
    runner: &csdk::Runner,
    progress: &dyn Fn(&str),
) -> Result<ModelBuildReport> {
    if !csdk::is_csdk_dir(csdk_dir) {
        return Err(Error::msg(format!(
            "CSDK wasn't found at {} (game\\bin\\win64\\resourcecompiler.exe is missing)",
            csdk_dir.display()
        )));
    }
    let (dir, name) = split_model(model)?;
    let original = project_dir.join("original");
    let template = native(&original, &format!("{dir}/{name}.vmdl"));
    let compiled_original = native(&original, &format!("{dir}/{name}.vmdl_c"));
    if !template.is_file() || !compiled_original.is_file() {
        return Err(Error::msg("This hero hasn't been extracted yet"));
    }

    let addon = csdk::addon_name(slug);
    let content_root = csdk_dir.join("content").join("citadel_addons").join(&addon);
    let game_root = csdk_dir.join("game").join("citadel_addons").join(&addon);
    for d in [&content_root, &game_root] {
        if d.exists() {
            std::fs::remove_dir_all(d).at(d)?;
        }
    }

    progress("Preparing the model source");
    let mut doc = kv3text::parse(&std::fs::read_to_string(&template).at(&template)?)?;
    use_backup_animations(&mut doc, model)?;
    let src = native(&content_root, &format!("{dir}/{name}.vmdl"));
    std::fs::create_dir_all(src.parent().unwrap()).at(&src)?;
    std::fs::write(&src, kv3text::to_string(&doc)).at(&src)?;
    for f in mesh_files(&doc) {
        let from = native(&original, &f);
        if !from.is_file() {
            return Err(Error::msg(format!("A mesh file the model needs is missing: {f}")));
        }
        let to = native(&content_root, &f);
        std::fs::create_dir_all(to.parent().unwrap()).at(&to)?;
        std::fs::copy(&from, &to).at(&to)?;
    }

    // What the compiler needs to resolve, from the game: the backup of the
    // original (animations) and everything the original refers to. These
    // are not packed (apart from the backup).
    progress("Copying what the compiler needs");
    let compiled = std::fs::read(&compiled_original).at(&compiled_original)?;
    let backup = format!("{}_c", backup_path(model)?);
    let mut placed: BTreeSet<String> = BTreeSet::new();
    let put = |rel: &str, bytes: &[u8]| -> Result<()> {
        let p = native(&game_root, rel);
        std::fs::create_dir_all(p.parent().unwrap()).at(&p)?;
        std::fs::write(&p, bytes).at(&p)
    };
    put(&backup, &compiled)?;
    let pak = steam::citadel_dir(game_dir).join("pak01_dir.vpk");
    let index = vpk::read_directory(&pak)?;
    for r in external_refs(&compiled)? {
        let rel = format!("{}_c", r.to_lowercase());
        if let Some(e) = index.find(&rel) {
            put(&rel, &index.extract(e)?)?;
            placed.insert(rel);
        }
    }

    progress("Compiling the model (this can take a few minutes)");
    let (ok, log) = runner(csdk_dir, &src)?;
    let out_rel = format!("{dir}/{name}.vmdl_c");
    let out_file = native(&game_root, &out_rel);
    if !ok || !out_file.is_file() {
        return Err(Error::msg(format!("CSDK couldn't compile the model. Is Steam running?\n\n{}", tail(&log))));
    }

    progress("Packing VPK");
    let mut packed: Vec<(String, Vec<u8>)> = vec![];
    for (rel, path) in files_under(&game_root)? {
        if placed.contains(&rel) {
            continue;
        }
        packed.push((rel, std::fs::read(&path).at(&path)?));
    }
    packed.sort_by(|a, b| a.0.cmp(&b.0));
    if let Some(p) = out_vpk.parent() {
        std::fs::create_dir_all(p).at(p)?;
    }
    vpk::write_vpk(out_vpk, &packed)?;
    let warnings = log
        .lines()
        .filter(|l| {
            let u = l.to_uppercase();
            u.contains("WARNING") || u.contains("ERROR")
        })
        .map(|l| l.trim().to_string())
        .take(50)
        .collect();
    Ok(ModelBuildReport {
        vpk: out_vpk.display().to_string(),
        vpk_size: std::fs::metadata(out_vpk).at(out_vpk)?.len(),
        files: packed.into_iter().map(|(p, _)| p).collect(),
        warnings,
        log: tail(&log),
    })
}

/// Every file under `root`, as (lowercase game path, real path).
fn files_under(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).at(&d)?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(root) {
                let rel = rel.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect::<Vec<_>>().join("/");
                out.push((rel, p));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VMDL: &str = r#"<!-- kv3 encoding:text:version{e21c7f3c-8a33-41c5-9977-a76d3a32aa0d} format:modeldoc28:version{fb63b6ca-f435-4aa0-a2c7-c66ddc651dca} -->
{
	rootNode =
	{
		_class = "RootNode"
		children =
		[
			{
				_class = "RenderMeshList"
				children =
				[
					{
						_class = "RenderMeshFile"
						name = "body"
						filename = "models/heroes_staging/hero/hero_body.dmx"
					},
				]
			},
			{
				_class = "PhysicsShapeList"
				children =
				[
					{
						_class = "PhysicsHullFile"
						filename = "models/heroes_staging/hero/hero_hull.dmx"
					},
				]
			},
			{
				_class = "AnimationList"
				default_root_bone_name = ""
				children =
				[
					{
						_class = "AnimFile"
						name = "idle"
						source_filename = "models/heroes_staging/hero/hero_idle.dmx"
					},
				]
			},
			{
				_class = "NmSkeletonList"
				children =
				[
					{
						_class = "NmSkeletonReference"
						filename = "models/heroes_staging/hero/hero.vnmskel"
					},
				]
			},
		]
	}
}
"#;

    #[test]
    fn finds_meshes_and_swaps_animations() {
        let mut doc = kv3text::parse(VMDL).unwrap();
        assert_eq!(mesh_files(&doc), vec!["models/heroes_staging/hero/hero_body.dmx", "models/heroes_staging/hero/hero_hull.dmx"]);
        use_backup_animations(&mut doc, "models/heroes_staging/hero/hero.vmdl").unwrap();
        let text = kv3text::to_string(&doc);
        assert!(text.contains("models/heroes_staging/hero/hero_backup.vmdl"));
        assert!(!text.contains("hero_idle.dmx"));
        assert!(text.contains("default_root_bone_name"));
    }

    #[test]
    fn model_paths() {
        assert_eq!(backup_path("models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner.vmdl").unwrap(), "models/heroes_staging/gigawatt_prisoner/gigawatt_prisoner_backup.vmdl");
        assert!(split_model("models/../x.vmdl").is_err());
        assert!(split_model("sounds/x.vmdl").is_err());
        assert!(split_model("models/x.vmat").is_err());
    }

    /// Fake tools: the "decompiler" writes the sample source, a mesh and a
    /// glb; the "compiler" writes a vmdl_c where the real one would.
    #[test]
    fn extract_and_build_end_to_end() {
        let t = tempfile::tempdir().unwrap();
        let model = "models/heroes_staging/hero/hero.vmdl";
        // Fake game: the compiled model (with a material and skeleton ref) + deps
        let game = t.path().join("game");
        std::fs::create_dir_all(steam::citadel_dir(&game)).unwrap();
        let compiled = fake_compiled(&["models/heroes_staging/hero/hero.vnmskel", "materials/hero.vmat"]);
        let pak = vpk::build_vpk(&[
            ("models/heroes_staging/hero/hero.vmdl_c".into(), compiled.clone()),
            ("models/heroes_staging/hero/hero.vnmskel_c".into(), b"skel".to_vec()),
            ("materials/hero.vmat_c".into(), b"mat".to_vec()),
        ])
        .unwrap();
        std::fs::write(steam::citadel_dir(&game).join("pak01_dir.vpk"), pak).unwrap();

        let s2v = t.path().join("s2v.exe");
        let decompiler = |_: &Path, args: &[String]| -> Result<(bool, String)> {
            let out = PathBuf::from(&args[args.iter().position(|a| a == "-o").unwrap() + 1]);
            if args.iter().any(|a| a == "--gltf_export_format") {
                std::fs::write(&out, b"glTF").unwrap();
            } else {
                assert!(args[3].contains("materials/hero.vmat_c"), "materials decompiled too: {}", args[3]);
                let d = out.join("models/heroes_staging/hero");
                std::fs::create_dir_all(d.join("clips")).unwrap();
                std::fs::write(d.join("hero.vmdl"), VMDL).unwrap();
                for f in ["hero_body.dmx", "hero_hull.dmx", "hero_idle.dmx"] {
                    std::fs::write(d.join(f), b"dmx").unwrap();
                }
            }
            Ok((true, "ok".into()))
        };
        let project = t.path().join("project");
        let r = extract(&s2v, &game, model, &project, &decompiler, &|_| {}).unwrap();
        assert_eq!(r.mesh_files.len(), 2);
        let folder = project.join("original/models/heroes_staging/hero");
        assert!(!folder.join("hero_idle.dmx").exists(), "animation DMX dropped");
        assert!(!folder.join("clips").exists());
        assert!(folder.join("hero_body.dmx").exists() && folder.join("hero.vmdl_c").exists());
        assert!(project.join("blender/hero.glb").exists() && project.join("return").is_dir());

        // Build
        let csdk_dir = t.path().join("csdk");
        std::fs::create_dir_all(csdk_dir.join("content")).unwrap();
        std::fs::create_dir_all(csdk::compiler_path(&csdk_dir).parent().unwrap()).unwrap();
        std::fs::write(csdk::compiler_path(&csdk_dir), b"").unwrap();
        let compiler = |csdk_dir: &Path, file: &Path| -> Result<(bool, String)> {
            let rel = file.strip_prefix(csdk_dir.join("content")).unwrap();
            let text = std::fs::read_to_string(file).unwrap();
            assert!(text.contains("AnimIncludeModel"));
            let out = csdk_dir.join("game").join(rel).with_extension("vmdl_c");
            assert!(out.parent().unwrap().join("hero_backup.vmdl_c").exists(), "backup is in place before compiling");
            assert!(out.parent().unwrap().join("hero.vnmskel_c").exists(), "deps are in place before compiling");
            std::fs::write(&out, b"compiled").unwrap();
            Ok((true, "WARNING: something minor\nOK: 1 compiled, 0 failed".into()))
        };
        let out = t.path().join("out/hero.vpk");
        let b = build(&csdk_dir, &game, model, &project, "Seven test", &out, &compiler, &|_| {}).unwrap();
        assert_eq!(b.files, vec!["models/heroes_staging/hero/hero.vmdl_c", "models/heroes_staging/hero/hero_backup.vmdl_c"]);
        assert_eq!(b.warnings, vec!["WARNING: something minor"]);
        let packed = vpk::read_directory(&out).unwrap();
        assert_eq!(packed.extract(packed.find("models/heroes_staging/hero/hero_backup.vmdl_c").unwrap()).unwrap(), compiled);
        assert!(csdk_dir.join("content/citadel_addons/dms_seven_test/models/heroes_staging/hero/hero_body.dmx").exists());
    }

    /// A minimal compiled resource: a DATA block (empty KV3 object) and a
    /// RERL block listing `refs`.
    fn fake_compiled(refs: &[&str]) -> Vec<u8> {
        // RERL: offset(8) count, entries of 16 bytes (id u64, name offset u32, pad), names.
        let mut rerl = vec![];
        rerl.extend(8u32.to_le_bytes());
        rerl.extend((refs.len() as u32).to_le_bytes());
        let names_at = 8 + refs.len() * 16;
        let mut names = vec![];
        for (i, r) in refs.iter().enumerate() {
            let entry = 8 + i * 16;
            rerl.extend(0u64.to_le_bytes());
            let rel = (names_at + names.len()) - (entry + 8);
            rerl.extend((rel as u32).to_le_bytes());
            rerl.extend(0u32.to_le_bytes());
            names.extend(r.as_bytes());
            names.push(0);
        }
        rerl.extend(names);
        let data = kv3_empty_object();
        // Header: size, hv 12, ver 0, block offset 8, count 2; table; blocks.
        let table_at = 16;
        let blocks_at = table_at + 24;
        let mut out = vec![];
        let total = blocks_at + rerl.len() + data.len();
        out.extend((total as u32).to_le_bytes());
        out.extend(12u16.to_le_bytes());
        out.extend(0u16.to_le_bytes());
        out.extend(8u32.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        for (i, (tag, off, len)) in [(b"RERL", blocks_at, rerl.len()), (b"DATA", blocks_at + rerl.len(), data.len())].iter().enumerate() {
            let entry = table_at + i * 12;
            out.extend(*tag);
            out.extend(((off - (entry + 4)) as u32).to_le_bytes());
            out.extend((*len as u32).to_le_bytes());
        }
        out.extend(rerl);
        out.extend(data);
        out
    }

    /// Binary KV3 v1 (uncompressed) holding `{}`.
    fn kv3_empty_object() -> Vec<u8> {
        let mut v = vec![];
        v.extend(0x4B56_3301u32.to_le_bytes()); // "KV3" v1
        v.extend([0u8; 16]); // format guid
        v.extend(0u32.to_le_bytes()); // method: none
        v.extend(0i32.to_le_bytes()); // bytes
        v.extend(2i32.to_le_bytes()); // ints: string count + object length
        v.extend(0i32.to_le_bytes()); // doubles
        let mut body = vec![];
        body.extend(0u32.to_le_bytes()); // 0 strings
        body.extend(0u32.to_le_bytes()); // object with 0 members
        body.push(9); // type: object
        body.extend(0xFFEE_DD00u32.to_le_bytes()); // end marker
        v.extend((body.len() as i32).to_le_bytes()); // uncompressed size
        v.extend(body);
        v
    }
}
