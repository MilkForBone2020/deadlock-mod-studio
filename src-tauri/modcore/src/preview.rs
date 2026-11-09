//! What the in-app preview needs to draw a hero roughly as the game does:
//! which glTF files to load, and each material's look settings read from
//! the extracted `.vmat` sources (outline colour, glow, shadowing and metal
//! maps). Colour textures come from the glTF itself, so repaints show.

use crate::{Error, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PreviewMaterial {
    /// Solid outline colour (sRGB 0..1).
    pub outline: Option<[f32; 3]>,
    /// Glow: tint (sRGB 0..1) and strength, with the mask texture if any.
    pub glow: Option<Glow>,
    /// Texture files (paths inside the project's `original` folder). The
    /// colour is Valve's, for showing the original beside your model.
    pub color: Option<String>,
    pub ao: Option<String>,
    pub metal: Option<String>,
    pub roughness: Option<f32>,
    /// Made for this model, from the hero's main material.
    pub is_new: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Glow {
    pub tint: [f32; 3],
    pub scale: f32,
    pub mask: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewInfo {
    /// The model to show: the latest sent from Blender, else the original.
    pub model: String,
    /// When `model` was sent from Blender (None: the original).
    pub sent: Option<String>,
    /// Has the hero's animations (the original's Blender file).
    pub original: String,
    /// By material name as in the glTF (hero materials by their base name).
    pub materials: BTreeMap<String, PreviewMaterial>,
}

fn kv(vmat: &str) -> BTreeMap<String, String> {
    vmat.lines()
        .filter_map(|l| {
            let t = l.trim();
            if !t.starts_with('"') || t.matches('"').count() != 4 {
                return None;
            }
            let mut p = t.split('"').filter(|s| !s.trim().is_empty());
            Some((p.next()?.to_string(), p.next()?.to_string()))
        })
        .collect()
}

fn vec3(v: &str) -> Option<[f32; 3]> {
    let n: Vec<f32> = v.trim_matches(|c| c == '[' || c == ']').split_whitespace().filter_map(|x| x.parse().ok()).collect();
    (n.len() >= 3).then(|| [n[0], n[1], n[2]])
}

fn uv_texture(original: &Path, v: Option<&String>) -> Option<String> {
    let v = v?;
    let p = v.split('/').fold(original.to_path_buf(), |p, s| p.join(s));
    // Flat 1×1 images and CSDK defaults add nothing to a preview.
    let (w, h) = image::image_dimensions(&p).ok()?;
    (w.max(h) > 4).then(|| v.clone())
}

/// Look settings of one material source.
pub fn material(vmat: &str, original: &Path) -> PreviewMaterial {
    let k = kv(vmat);
    let on = |key: &str| k.get(key).map(|v| v != "0").unwrap_or(false);
    let glow = on("F_SELF_ILLUM").then(|| Glow {
        tint: k.get("g_vSelfIllumTint1").and_then(|v| vec3(v)).unwrap_or([1.0; 3]),
        scale: k.get("g_flSelfIllumScale1").and_then(|v| v.parse().ok()).unwrap_or(1.0),
        mask: uv_texture(original, k.get("TextureSelfIllumMask1")),
    });
    PreviewMaterial {
        outline: on("F_SOLID_COLOR_OUTLINE").then(|| k.get("g_vSolidOutlineTint").and_then(|v| vec3(v)).unwrap_or([0.05; 3])),
        glow,
        color: k.get("TextureColor1").filter(|v| v.split('/').fold(original.to_path_buf(), |p, s| p.join(s)).is_file()).cloned(),
        ao: uv_texture(original, k.get("TextureAmbientOcclusion1")),
        metal: uv_texture(original, k.get("TextureMetalness1")),
        roughness: k.get("TextureRoughness1").and_then(|v| vec3(v)).map(|v| v[0]),
        is_new: false,
    }
}

fn read_materials(returned: &Path) -> Vec<(String, Option<f32>)> {
    std::fs::read(returned.join("manifest.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<crate::blender::Returned>(&b).ok())
        .map(|m| m.material_files.into_iter().map(|f| (f.name, f.roughness)).collect())
        .unwrap_or_default()
}

/// Gather the preview for a project. `materials` are the hero's `.vmat`
/// paths (from summary.json); `returned` the latest folder sent from Blender.
pub fn info(project_dir: &Path, model: &str, materials: &[String], returned: Option<&Path>) -> Result<PreviewInfo> {
    let name = model.trim_end_matches(".vmdl").rsplit('/').next().unwrap_or(model);
    let original_glb = project_dir.join("blender").join(format!("{name}.glb"));
    if !original_glb.is_file() {
        return Err(Error::msg("Extract the hero first"));
    }
    let original = project_dir.join("original");
    let stem = |m: &str| m.rsplit('/').next().unwrap_or(m).trim_end_matches(".vmat").to_string();
    let mut out = BTreeMap::new();
    let mut main = None;
    for m in materials {
        let src: PathBuf = m.split('/').fold(original.clone(), |p, s| p.join(s));
        let Ok(text) = std::fs::read_to_string(&src) else { continue };
        let pm = material(&text, &original);
        if stem(m) == name || main.is_none() {
            main = Some(pm.clone());
        }
        out.insert(stem(m), pm);
    }
    let (model_path, sent) = match returned {
        Some(r) => {
            let manifest: crate::blender::Returned = serde_json::from_slice(&std::fs::read(r.join("manifest.json")).map_err(|e| Error::msg(e.to_string()))?)?;
            // New materials look like the hero's main one, minus the maps
            // drawn for the hero's UVs and the glow (as the build makes them).
            for (mat, rough) in read_materials(r) {
                let b = crate::meshswap::base_material_name(&mat);
                if out.contains_key(b) {
                    continue;
                }
                let hero = materials.iter().any(|m| stem(m) == b);
                let base = main.clone().unwrap_or_default();
                // A hero material whose source wasn't extracted: outline only.
                out.insert(if hero { b.to_string() } else { mat }, PreviewMaterial { outline: base.outline, glow: None, color: None, ao: None, metal: None, roughness: rough.or(base.roughness), is_new: !hero });
            }
            (r.join(&manifest.glb), Some(manifest.sent))
        }
        None => (original_glb.clone(), None),
    };
    Ok(PreviewInfo { model: model_path.display().to_string(), sent, original: original_glb.display().to_string(), materials: out })
}

/// A file the preview may read: the project's glTF files and textures in
/// its `original` folder, nothing else.
pub fn allowed_file(project_dir: &Path, path: &Path) -> bool {
    let Ok(p) = path.canonicalize() else { return false };
    ["blender", "return", "original"].iter().any(|d| project_dir.join(d).canonicalize().map(|root| p.starts_with(root)).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_look_settings() {
        let t = tempfile::tempdir().unwrap();
        let o = t.path();
        std::fs::create_dir_all(o.join("m")).unwrap();
        image::GrayImage::new(64, 64).save(o.join("m/ao.png")).unwrap();
        image::GrayImage::new(1, 1).save(o.join("m/flat.png")).unwrap();
        let vmat = "\"Layer0\"\n{\n\t\"F_SELF_ILLUM\"\t\"1\"\n\t\"F_SOLID_COLOR_OUTLINE\"\t\"1\"\n\t\"g_flSelfIllumScale1\"\t\"10\"\n\t\"g_vSelfIllumTint1\"\t\"[0.749020 0.384314 0.043137 0.000000]\"\n\t\"g_vSolidOutlineTint\"\t\"[0.062745 0.019608 0.019608 0.000000]\"\n\t\"TextureRoughness1\"\t\"[0.972549 0.972549 0.972549 0.000000]\"\n\t\"TextureAmbientOcclusion1\"\t\"m/ao.png\"\n\t\"TextureMetalness1\"\t\"m/flat.png\"\n\t\"TextureSelfIllumMask1\"\t\"m/missing.png\"\n}\n";
        let m = material(vmat, o);
        assert_eq!(m.outline, Some([0.062745, 0.019608, 0.019608]));
        let g = m.glow.unwrap();
        assert_eq!(g.scale, 10.0);
        assert_eq!(g.mask, None);
        assert_eq!(m.ao.as_deref(), Some("m/ao.png"));
        assert_eq!(m.color, None, "no colour file extracted");
        assert_eq!(m.metal, None, "a flat texture adds nothing");
        assert!((m.roughness.unwrap() - 0.972549).abs() < 1e-6);
    }

    #[test]
    fn only_project_files() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("original/m")).unwrap();
        std::fs::write(t.path().join("original/m/a.png"), b"x").unwrap();
        std::fs::write(t.path().join("secret.txt"), b"x").unwrap();
        assert!(allowed_file(t.path(), &t.path().join("original/m/a.png")));
        assert!(!allowed_file(t.path(), &t.path().join("original/../secret.txt")));
        assert!(!allowed_file(t.path(), &t.path().join("secret.txt")));
    }
}
