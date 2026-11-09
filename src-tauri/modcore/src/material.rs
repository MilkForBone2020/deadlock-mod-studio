//! Hero materials changed in Blender: a repainted colour texture becomes a
//! copy of the original material (Valve's hero shader and every setting
//! kept) at the original's path, with the new image in its colour slot.
//!
//! The decompiled materials are the older `"Layer0" { "key" "value" }`
//! text format; texture entries point at source images (PNG/TGA) that the
//! compiler turns into `.vtex_c` files.

use crate::{Error, IoCtx, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MaterialResult {
    pub material: String,
    /// Game path of the material rebuilt, e.g. `models/.../x.vmat`.
    pub path: String,
    pub color: Option<String>,
    pub size: (u32, u32),
    /// The image wasn't a power of two and was resized.
    pub resized: bool,
}

fn native(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(root.to_path_buf(), |p, s| p.join(s))
}

/// Split a `"key" "value"` line; None for anything else.
fn kv_line(line: &str) -> Option<(String, String)> {
    let t = line.trim();
    let mut parts = t.split('"').filter(|s| !s.trim().is_empty());
    let k = parts.next()?.to_string();
    let v = parts.next()?.to_string();
    if t.starts_with('"') && t.matches('"').count() == 4 {
        Some((k, v))
    } else {
        None
    }
}

fn is_file_ref(v: &str) -> bool {
    let l = v.to_lowercase();
    [".png", ".tga", ".psd", ".jpg", ".jpeg", ".tif", ".tiff", ".exr", ".pfm"].iter().any(|e| l.ends_with(e))
}

/// Rewrite a material source with `color` as its colour texture. Returns the
/// new text and every texture file it refers to.
pub fn with_color(vmat: &str, color: &str) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut refs = vec![];
    let old_color = vmat.lines().filter_map(kv_line).find(|(k, _)| k == "TextureColor1").map(|(_, v)| v);
    let mut depth_skip: Option<i32> = None;
    let mut depth = 0i32;
    for line in vmat.lines() {
        let t = line.trim();
        // Drop the "Compiled Textures" block: the compiler writes its own.
        if depth_skip.is_none() && t == "\"Compiled Textures\"" {
            depth_skip = Some(depth);
            continue;
        }
        if t == "{" {
            depth += 1;
        } else if t == "}" {
            depth -= 1;
        }
        if let Some(d) = depth_skip {
            if t == "}" && depth == d {
                depth_skip = None;
            }
            continue;
        }
        if let Some((k, v)) = kv_line(line) {
            let replace = k == "TextureColor1" || (k.starts_with("TextureNprTra") && old_color.as_deref() == Some(v.as_str()));
            let v2 = if replace { color.to_string() } else { v.clone() };
            if is_file_ref(&v2) {
                refs.push(v2.clone());
            }
            if replace {
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                out.push_str(&format!("{indent}\"{k}\"\t\"{v2}\"\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    refs.sort();
    refs.dedup();
    (out, refs)
}

/// Decode an image, resize it to a power of two if needed, drop alpha, and
/// encode as PNG. Returns (png, size, resized).
pub fn prepare_color(bytes: &[u8]) -> Result<(Vec<u8>, (u32, u32), bool)> {
    let img = image::load_from_memory(bytes).map_err(|e| Error::msg(format!("Couldn't read the texture: {e}")))?;
    let (w, h) = (img.width(), img.height());
    let p2 = |n: u32| -> u32 {
        let up = n.next_power_of_two();
        let down = up / 2;
        if down >= 1 && n - down < up - n { down } else { up }.clamp(4, 4096)
    };
    let (nw, nh) = (p2(w), p2(h));
    let resized = (nw, nh) != (w, h);
    let img = if resized { img.resize_exact(nw, nh, image::imageops::FilterType::Lanczos3) } else { img };
    let rgb = img.to_rgb8();
    let mut out = std::io::Cursor::new(Vec::new());
    rgb.write_to(&mut out, image::ImageFormat::Png).map_err(|e| Error::msg(format!("Couldn't write the texture: {e}")))?;
    Ok((out.into_inner(), (nw, nh), resized))
}

/// Apply the materials changed in Blender. `materials` are the hero's
/// material paths (`.vmat`); sources come from `original`.
pub fn apply(
    returned: &Path,
    files: &[crate::blender::MaterialFile],
    materials: &[String],
    original: &Path,
    content_root: &Path,
) -> Result<Vec<MaterialResult>> {
    let mut results = vec![];
    for f in files {
        let Some(color_rel) = &f.color else { continue };
        let base = crate::meshswap::base_material_name(&f.name);
        let path = materials
            .iter()
            .find(|m| m.rsplit('/').next().unwrap_or(m).trim_end_matches(".vmat") == base)
            .ok_or_else(|| Error::msg(format!("Material \"{}\" isn't one of the hero's materials (new materials aren't supported yet)", f.name)))?;
        let src = native(original, path);
        let text = std::fs::read_to_string(&src)
            .map_err(|_| Error::msg(format!("The source of {path} wasn't extracted. Extract the hero again.")))?;
        let img_path = returned.join(color_rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        let (png, size, resized) = prepare_color(&std::fs::read(&img_path).at(&img_path)?)?;
        let (dir, stem) = path.trim_end_matches(".vmat").rsplit_once('/').unwrap_or(("materials", path));
        let new_color = format!("{dir}/{stem}_dms_color.png");
        let (vmat, refs) = with_color(&text, &new_color);
        // The new image, then every other texture the material uses.
        let out = native(content_root, &new_color);
        std::fs::create_dir_all(out.parent().unwrap()).at(&out)?;
        std::fs::write(&out, png).at(&out)?;
        for r in refs.iter().filter(|r| **r != new_color) {
            let from = native(original, r);
            let to = native(content_root, r);
            if from.is_file() {
                std::fs::create_dir_all(to.parent().unwrap()).at(&to)?;
                std::fs::copy(&from, &to).at(&to)?;
            } else if !r.starts_with("materials/default/") {
                return Err(Error::msg(format!("{path} uses {r}, which wasn't extracted. Extract the hero again.")));
            }
            // materials/default/... ships with CSDK.
        }
        let vout = native(content_root, path);
        std::fs::create_dir_all(vout.parent().unwrap()).at(&vout)?;
        std::fs::write(&vout, vmat).at(&vout)?;
        results.push(MaterialResult { material: base.to_string(), path: path.clone(), color: Some(new_color), size, resized });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VMAT: &str = "\"Layer0\"\n{\n\t\"shader\"\t\"pbr.vfx\"\n\t\"F_SELF_ILLUM\"\t\"1\"\n\t\"TextureRoughness1\"\t\"[0.972549 0.972549 0.972549 0.000000]\"\n\t\"TextureAltColor1\"\t\"materials/default/default_color.tga\"\n\t\"TextureColor1\"\t\"models/h/materials/h_color.png\"\n\t\"TextureNprTramsissiveColor1\"\t\"models/h/materials/h_color.png\"\n\t\"TextureSelfIllumMask1\"\t\"models/h/materials/h_emissive.png\"\n\t\"Compiled Textures\"\n\t{\n\t\t\"g_tColor\"\t\"models/h/materials/h_color_png_1.vtex\"\n\t}\n}\n";

    #[test]
    fn swaps_colour_and_keeps_the_rest() {
        let (out, refs) = with_color(VMAT, "models/h/materials/h_dms_color.png");
        assert!(out.contains("\"TextureColor1\"\t\"models/h/materials/h_dms_color.png\""));
        assert!(out.contains("\"TextureNprTramsissiveColor1\"\t\"models/h/materials/h_dms_color.png\""), "transmissive followed the colour");
        assert!(out.contains("\"F_SELF_ILLUM\"\t\"1\"") && out.contains("pbr.vfx"));
        assert!(!out.contains("Compiled Textures") && !out.contains(".vtex"));
        assert!(out.trim_end().ends_with('}'));
        assert_eq!(out.matches('{').count(), out.matches('}').count());
        assert_eq!(refs, vec!["materials/default/default_color.tga", "models/h/materials/h_dms_color.png", "models/h/materials/h_emissive.png"]);
    }

    #[test]
    fn textures_become_power_of_two_rgb() {
        let img = image::RgbaImage::from_pixel(1000, 512, image::Rgba([10, 200, 30, 0]));
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let (out, size, resized) = prepare_color(png.get_ref()).unwrap();
        assert_eq!(size, (1024, 512));
        assert!(resized);
        let back = image::load_from_memory(&out).unwrap();
        assert_eq!(back.color(), image::ColorType::Rgb8);
        assert_eq!(back.to_rgb8().get_pixel(5, 5).0, [10, 200, 30]);
    }

    #[test]
    fn applies_to_a_project() {
        let t = tempfile::tempdir().unwrap();
        let original = t.path().join("original");
        let content = t.path().join("content");
        let ret = t.path().join("ret");
        std::fs::create_dir_all(original.join("models/h/materials")).unwrap();
        std::fs::create_dir_all(ret.join("textures")).unwrap();
        std::fs::write(original.join("models/h/materials/h.vmat"), VMAT).unwrap();
        std::fs::write(original.join("models/h/materials/h_emissive.png"), b"e").unwrap();
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]));
        img.save(ret.join("textures/h_color.png")).unwrap();
        let files = vec![
            crate::blender::MaterialFile { name: "h.001".into(), color: Some("textures/h_color.png".into()) },
            crate::blender::MaterialFile { name: "other".into(), color: None },
        ];
        let r = apply(&ret, &files, &["models/h/materials/h.vmat".to_string()], &original, &content).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].path, "models/h/materials/h.vmat");
        assert!(content.join("models/h/materials/h.vmat").is_file());
        assert!(content.join("models/h/materials/h_dms_color.png").is_file());
        assert!(content.join("models/h/materials/h_emissive.png").is_file());
        let bad = vec![crate::blender::MaterialFile { name: "new_mat".into(), color: Some("textures/h_color.png".into()) }];
        assert!(apply(&ret, &bad, &["models/h/materials/h.vmat".to_string()], &original, &content).is_err());
    }
}
