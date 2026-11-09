//! Hero materials changed in Blender: a repainted colour texture becomes a
//! copy of the original material (Valve's hero shader and every setting
//! kept) at the original's path, with the new image in its colour slot.
//!
//! The decompiled materials are the older `"Layer0" { "key" "value" }`
//! text format; texture entries point at source images (PNG/TGA) that the
//! compiler turns into `.vtex_c` files.
//!
//! Materials that aren't the hero's are new: they start from the hero's main
//! material (hero shader, outline, rim light and status effects kept) with
//! the new colour, and any texture drawn for the hero's UV layout replaced
//! by a neutral one.

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
    /// Made for this model (not one of the hero's materials).
    #[serde(rename = "new")]
    pub is_new: bool,
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

enum Edit {
    Keep,
    Set(String),
    Drop,
}

/// Rewrite a material source line by line. `edit` decides each
/// `"key" "value"` entry; `extra` entries are added at the end of the
/// layer. The "Compiled Textures" block is dropped (the compiler writes its
/// own). Returns the new text and every texture file it refers to.
fn rewrite(vmat: &str, mut edit: impl FnMut(&str, &str) -> Edit, extra: &[(String, String)]) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut refs = vec![];
    let mut depth_skip: Option<i32> = None;
    let mut depth = 0i32;
    let mut indent = String::from("\t");
    for line in vmat.lines() {
        let t = line.trim();
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
        if t == "}" && depth == 0 {
            for (k, v) in extra {
                if is_file_ref(v) {
                    refs.push(v.clone());
                }
                out.push_str(&format!("{indent}\"{k}\"\t\"{v}\"\n"));
            }
        }
        if let Some((k, v)) = kv_line(line) {
            indent = line.chars().take_while(|c| c.is_whitespace()).collect();
            let v2 = match edit(&k, &v) {
                Edit::Drop => continue,
                Edit::Keep => v,
                Edit::Set(n) => n,
            };
            if is_file_ref(&v2) {
                refs.push(v2.clone());
            }
            out.push_str(&format!("{indent}\"{k}\"\t\"{v2}\"\n"));
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    refs.sort();
    refs.dedup();
    (out, refs)
}

fn old_color(vmat: &str) -> Option<String> {
    vmat.lines().filter_map(kv_line).find(|(k, _)| k == "TextureColor1").map(|(_, v)| v)
}

/// Rewrite a material source with `color` as its colour texture. Returns the
/// new text and every texture file it refers to.
pub fn with_color(vmat: &str, color: &str) -> (String, Vec<String>) {
    let old = old_color(vmat);
    rewrite(
        vmat,
        |k, v| {
            if k == "TextureColor1" || (k.starts_with("TextureNprTra") && old.as_deref() == Some(v)) {
                Edit::Set(color.to_string())
            } else {
                Edit::Keep
            }
        },
        &[],
    )
}

/// What a new material changes in the hero material it starts from.
pub struct NewMaterial<'a> {
    pub color: &'a str,
    pub roughness: Option<f32>,
    /// Metalness texture; None for the plain default (not metal).
    pub metal: Option<&'a str>,
}

/// A CSDK default for a texture slot laid out for the hero's UVs.
fn default_for(key: &str) -> Option<&'static str> {
    let k = key.to_lowercase();
    if k.starts_with("textureambientocclusion") {
        Some("materials/default/default_ao.tga")
    } else if k.starts_with("texturenormal") {
        Some("materials/default/default_normal.tga")
    } else if k.starts_with("texturemetalness") {
        Some("materials/default/default_metal.tga")
    } else {
        None
    }
}

/// Turn a hero material into a new one: same shader and look settings, the
/// new colour, no glow, no vertex-colour tint, and every texture drawn for
/// the hero's UV layout (`uv_specific`) swapped for a neutral one. For slots
/// without a CSDK default, `flat` makes a replacement (a flat image) and
/// returns its path.
pub fn from_template(
    vmat: &str,
    m: &NewMaterial,
    uv_specific: impl Fn(&str) -> bool,
    mut flat: impl FnMut(&str, &str) -> Option<String>,
) -> (String, Vec<String>) {
    let old = old_color(vmat);
    let mut seen_roughness = false;
    let mut seen_metal = false;
    let fmt4 = |r: f32| format!("[{r:.6} {r:.6} {r:.6} 0.000000]");
    let (mut out, mut refs) = rewrite(
        vmat,
        |k, v| {
            let kl = k.to_lowercase();
            if k == "TextureColor1" || (k.starts_with("TextureNprTra") && old.as_deref() == Some(v)) {
                return Edit::Set(m.color.to_string());
            }
            if kl.starts_with("f_self_illum") {
                return Edit::Set("0".into());
            }
            if kl.contains("selfillum") {
                return Edit::Drop;
            }
            if kl.starts_with("g_fvertexcolorstrength") {
                return Edit::Set("0".into());
            }
            if kl.starts_with("textureroughness") {
                seen_roughness = true;
                return match m.roughness {
                    Some(r) => Edit::Set(fmt4(r.clamp(0.0, 1.0))),
                    None => Edit::Keep,
                };
            }
            if kl.starts_with("texturemetalness") {
                seen_metal = true;
                return Edit::Set(m.metal.unwrap_or("materials/default/default_metal.tga").to_string());
            }
            if is_file_ref(v) && !v.starts_with("materials/default/") && uv_specific(v) {
                if let Some(d) = default_for(k) {
                    return Edit::Set(d.into());
                }
                if let Some(f) = flat(k, v) {
                    return Edit::Set(f);
                }
            }
            Edit::Keep
        },
        &[],
    );
    // Slots the hero material didn't have.
    let mut extra = vec![];
    if !seen_roughness {
        if let Some(r) = m.roughness {
            extra.push(("TextureRoughness1".to_string(), fmt4(r.clamp(0.0, 1.0))));
        }
    }
    if !seen_metal {
        if let Some(t) = m.metal {
            extra.push(("TextureMetalness1".to_string(), t.to_string()));
        }
    }
    if !extra.is_empty() {
        let (o, r) = rewrite(&out, |_, _| Edit::Keep, &extra);
        out = o;
        refs.extend(r);
        refs.sort();
        refs.dedup();
    }
    (out, refs)
}

fn linear_to_srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

/// A 4×4 PNG of one colour.
pub fn flat_png(rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(4, 4, image::Rgb(rgb));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).expect("png encode");
    out.into_inner()
}

/// The average colour of an image file.
fn average(path: &Path) -> Option<[u8; 3]> {
    let img = image::open(path).ok()?.resize_exact(1, 1, image::imageops::FilterType::Triangle).to_rgb8();
    Some(img.get_pixel(0, 0).0)
}

/// A material name made safe for a file name.
pub fn safe_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() { "material".into() } else { s }
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

/// What `apply` did: the materials rebuilt or created, and the `.vmat` path
/// of each new material by its Blender name.
#[derive(Default)]
pub struct Applied {
    pub results: Vec<MaterialResult>,
    pub new_paths: std::collections::HashMap<String, String>,
}

fn write_file(root: &Path, rel: &str, bytes: &[u8]) -> Result<()> {
    let out = native(root, rel);
    std::fs::create_dir_all(out.parent().unwrap()).at(&out)?;
    std::fs::write(&out, bytes).at(&out)
}

/// Copy the textures a material uses from the extracted originals (CSDK has
/// `materials/default/...`; files written already are left alone).
fn copy_refs(refs: &[String], written: &[String], original: &Path, content_root: &Path, path: &str) -> Result<()> {
    for r in refs.iter().filter(|r| !written.contains(r)) {
        let from = native(original, r);
        let to = native(content_root, r);
        if from.is_file() {
            std::fs::create_dir_all(to.parent().unwrap()).at(&to)?;
            std::fs::copy(&from, &to).at(&to)?;
        } else if !r.starts_with("materials/default/") {
            return Err(Error::msg(format!("{path} uses {r}, which wasn't extracted. Extract the hero again.")));
        }
    }
    Ok(())
}

fn stem(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path).trim_end_matches(".vmat")
}

/// Apply the materials from Blender. Hero materials (`materials`, `.vmat`
/// paths) with a repainted colour texture are rebuilt at their own path.
/// Any other material is new: it starts from the hero's main material and is
/// written under `<model dir>/dms_materials/`. Sources come from `original`.
pub fn apply(
    returned: &Path,
    files: &[crate::blender::MaterialFile],
    materials: &[String],
    model: &str,
    original: &Path,
    content_root: &Path,
) -> Result<Applied> {
    let mut done = Applied::default();
    let (model_dir, model_name) = model.trim_end_matches(".vmdl").rsplit_once('/').unwrap_or(("models", model));
    let mut used_names: Vec<String> = vec![];
    for f in files {
        let base = crate::meshswap::base_material_name(&f.name);
        let hero = materials.iter().find(|m| stem(m) == base);
        let read_image = |rel: &str| -> Result<(Vec<u8>, (u32, u32), bool)> {
            let img_path = returned.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            prepare_color(&std::fs::read(&img_path).at(&img_path)?)
        };
        if let Some(path) = hero {
            let Some(color_rel) = &f.color else { continue };
            let src = native(original, path);
            let text = std::fs::read_to_string(&src)
                .map_err(|_| Error::msg(format!("The source of {path} wasn't extracted. Extract the hero again.")))?;
            let (png, size, resized) = read_image(color_rel)?;
            let (dir, st) = path.trim_end_matches(".vmat").rsplit_once('/').unwrap_or(("materials", path));
            let new_color = format!("{dir}/{st}_dms_color.png");
            let (vmat, refs) = with_color(&text, &new_color);
            write_file(content_root, &new_color, &png)?;
            copy_refs(&refs, std::slice::from_ref(&new_color), original, content_root, path)?;
            write_file(content_root, path, vmat.as_bytes())?;
            done.results.push(MaterialResult { material: base.to_string(), path: path.clone(), color: Some(new_color), size, resized, is_new: false });
            continue;
        }
        if f.name.is_empty() {
            continue;
        }

        // A new material, from the hero's main material.
        let template = materials
            .iter()
            .filter(|m| native(original, m).is_file())
            .find(|m| stem(m) == model_name)
            .or_else(|| materials.iter().find(|m| native(original, m).is_file()))
            .ok_or_else(|| Error::msg("None of the hero's materials were extracted. Extract the hero again."))?;
        let text = std::fs::read_to_string(native(original, template)).at(&native(original, template))?;
        let mut safe = safe_name(&f.name);
        let mut n = 2;
        while used_names.contains(&safe) || materials.iter().any(|m| stem(m).eq_ignore_ascii_case(&safe)) {
            safe = format!("{}_{n}", safe_name(&f.name));
            n += 1;
        }
        used_names.push(safe.clone());
        let dir = format!("{model_dir}/dms_materials");
        let path = format!("{dir}/{safe}.vmat");
        let color = format!("{dir}/{safe}_color.png");
        let (png, size, resized) = match &f.color {
            Some(rel) => read_image(rel)?,
            None => {
                let c = f.color_value.unwrap_or([0.8, 0.8, 0.8, 1.0]);
                (flat_png([linear_to_srgb(c[0]), linear_to_srgb(c[1]), linear_to_srgb(c[2])]), (4, 4), false)
            }
        };
        write_file(content_root, &color, &png)?;
        let mut written = vec![color.clone()];
        let metal = match f.metallic {
            Some(m) if m > 0.0 => {
                let rel = format!("{dir}/{safe}_metal.png");
                let v = (m.clamp(0.0, 1.0) * 255.0).round() as u8;
                write_file(content_root, &rel, &flat_png([v, v, v]))?;
                written.push(rel.clone());
                Some(rel)
            }
            _ => None,
        };
        let uv_specific = |v: &str| image::image_dimensions(native(original, v)).map(|(w, h)| w.max(h) > 4).unwrap_or(false);
        let mut flat_err = None;
        let (vmat, refs) = from_template(
            &text,
            &NewMaterial { color: &color, roughness: f.roughness, metal: metal.as_deref() },
            uv_specific,
            |k, v| {
                let avg = average(&native(original, v))?;
                let rel = format!("{dir}/{safe}_{}.png", safe_name(k));
                if let Err(e) = write_file(content_root, &rel, &flat_png(avg)) {
                    flat_err = Some(e);
                    return None;
                }
                written.push(rel.clone());
                Some(rel)
            },
        );
        if let Some(e) = flat_err {
            return Err(e);
        }
        copy_refs(&refs, &written, original, content_root, &path)?;
        write_file(content_root, &path, vmat.as_bytes())?;
        done.new_paths.insert(f.name.clone(), path.clone());
        done.results.push(MaterialResult { material: f.name.clone(), path, color: Some(color), size, resized, is_new: true });
    }
    Ok(done)
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

    fn mf(name: &str, color: Option<&str>) -> crate::blender::MaterialFile {
        crate::blender::MaterialFile { name: name.into(), color: color.map(|c| c.into()), ..Default::default() }
    }

    fn project() -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let original = t.path().join("original");
        let content = t.path().join("content");
        let ret = t.path().join("ret");
        std::fs::create_dir_all(original.join("models/h/materials")).unwrap();
        std::fs::create_dir_all(ret.join("textures")).unwrap();
        std::fs::write(original.join("models/h/materials/h.vmat"), VMAT).unwrap();
        image::GrayImage::from_pixel(64, 64, image::Luma([200])).save(original.join("models/h/materials/h_emissive.png")).unwrap();
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]));
        img.save(ret.join("textures/h_color.png")).unwrap();
        (t, original, content, ret)
    }

    #[test]
    fn applies_to_a_project() {
        let (_t, original, content, ret) = project();
        let files = vec![mf("h.001", Some("textures/h_color.png")), mf("h", None)];
        let mats = ["models/h/materials/h.vmat".to_string()];
        let r = apply(&ret, &files, &mats, "models/h/h.vmdl", &original, &content).unwrap();
        assert_eq!(r.results.len(), 1);
        assert!(r.new_paths.is_empty());
        assert_eq!(r.results[0].path, "models/h/materials/h.vmat");
        assert!(!r.results[0].is_new);
        assert!(content.join("models/h/materials/h.vmat").is_file());
        assert!(content.join("models/h/materials/h_dms_color.png").is_file());
        assert!(content.join("models/h/materials/h_emissive.png").is_file());
    }

    #[test]
    fn new_materials_start_from_the_hero() {
        let (_t, original, content, ret) = project();
        let mut plain = mf("Top Hat.001", None);
        plain.color_value = Some([1.0, 0.0, 0.0, 1.0]);
        plain.roughness = Some(0.25);
        plain.metallic = Some(1.0);
        let files = vec![mf("Band", Some("textures/h_color.png")), plain];
        let mats = ["models/h/materials/h.vmat".to_string()];
        let r = apply(&ret, &files, &mats, "models/h/h.vmdl", &original, &content).unwrap();
        assert_eq!(r.results.len(), 2);
        assert!(r.results.iter().all(|m| m.is_new));
        assert_eq!(r.new_paths["Band"], "models/h/dms_materials/band.vmat");
        assert_eq!(r.new_paths["Top Hat.001"], "models/h/dms_materials/top_hat_001.vmat");

        let hat = std::fs::read_to_string(content.join("models/h/dms_materials/top_hat_001.vmat")).unwrap();
        assert!(hat.contains("pbr.vfx"), "hero shader kept");
        assert!(hat.contains("\"TextureColor1\"\t\"models/h/dms_materials/top_hat_001_color.png\""));
        assert!(hat.contains("\"TextureNprTramsissiveColor1\"\t\"models/h/dms_materials/top_hat_001_color.png\""));
        assert!(hat.contains("\"F_SELF_ILLUM\"\t\"0\"") && !hat.contains("TextureSelfIllumMask") && !hat.contains("h_emissive"));
        assert!(hat.contains("\"TextureRoughness1\"\t\"[0.250000 0.250000 0.250000 0.000000]\""));
        assert!(hat.contains("\"TextureMetalness1\"\t\"models/h/dms_materials/top_hat_001_metal.png\""));
        assert!(!hat.contains("Compiled Textures"));
        assert_eq!(hat.matches('{').count(), hat.matches('}').count());
        let red = image::open(content.join("models/h/dms_materials/top_hat_001_color.png")).unwrap().to_rgb8();
        assert_eq!(red.get_pixel(0, 0).0, [255, 0, 0]);

        let band = std::fs::read_to_string(content.join("models/h/dms_materials/band.vmat")).unwrap();
        assert!(!band.contains("TextureMetalness1"), "no metal slot in the hero material, none added");
        assert!(content.join("models/h/dms_materials/band_color.png").is_file());
        assert!(!content.join("models/h/materials/h.vmat").exists(), "the hero material is untouched");
    }

    #[test]
    fn uv_specific_textures_become_neutral() {
        let vmat = "\"Layer0\"\n{\n\t\"shader\"\t\"pbr.vfx\"\n\t\"g_fVertexColorStrength1\"\t\"1\"\n\t\"TextureAmbientOcclusion1\"\t\"m/ao.png\"\n\t\"TextureNormal1\"\t\"m/nr.png\"\n\t\"TextureTintMask1\"\t\"m/tint.png\"\n\t\"TextureOther1\"\t\"m/small.png\"\n\t\"TextureColor1\"\t\"m/c.png\"\n}\n";
        let big = |v: &str| v != "m/small.png";
        let m = NewMaterial { color: "n/c.png", roughness: Some(0.5), metal: None };
        let (out, refs) = from_template(vmat, &m, big, |k, _| Some(format!("n/{}.png", safe_name(k))));
        assert!(out.contains("\"TextureAmbientOcclusion1\"\t\"materials/default/default_ao.tga\""));
        assert!(out.contains("\"TextureNormal1\"\t\"materials/default/default_normal.tga\""));
        assert!(out.contains("\"TextureTintMask1\"\t\"n/texturetintmask1.png\""));
        assert!(out.contains("\"TextureOther1\"\t\"m/small.png\""), "flat textures don't depend on UVs: kept");
        assert!(out.contains("\"g_fVertexColorStrength1\"\t\"0\""));
        assert!(out.contains("\"TextureRoughness1\"\t\"[0.500000 0.500000 0.500000 0.000000]\"\n}"), "added at the end: {out}");
        assert!(refs.contains(&"m/small.png".to_string()) && refs.contains(&"n/c.png".to_string()));
    }
}
