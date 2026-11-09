//! Hero portraits (the cards in the HUD's top bar and menus, plus the small,
//! minimap and tall icons): my pictures, fitted to each slot's size, written
//! as flat PSDs at the game's paths, compiled with CSDK and packed.
//!
//! The hero data names each image as `file://{images}/heroes/x_card.psd`;
//! the game loads the compiled `panorama/images/heroes/x_card_psd.vtex_c`.

use crate::csdk::{self, Runner};
use crate::{vpk, Error, IoCtx, Result};
use image::{imageops::FilterType, RgbaImage};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// (hero data field, label, width, height).
pub const SLOTS: &[(&str, &str, u32, u32)] = &[
    ("m_strIconHeroCard", "Card", 280, 380),
    ("m_strIconHeroCardCritical", "Card: low health", 280, 380),
    ("m_strIconHeroCardGloat", "Card: gloating", 280, 380),
    ("m_strTopBarVertical", "Tall icon", 120, 200),
    ("m_strIconImageSmall", "Small icon", 128, 128),
    ("m_strMinimapImage", "Minimap icon", 50, 50),
];

pub fn slot(field: &str) -> Option<(&'static str, u32, u32)> {
    SLOTS.iter().find(|s| s.0 == field).map(|s| (s.1, s.2, s.3))
}

/// One picture for one slot.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IconJob {
    pub field: String,
    pub source: String,
}

/// `file://{images}/heroes/x_card.psd` -> `panorama/images/heroes/x_card`
/// (the path without extension).
pub fn image_stem(hero_data_value: &str) -> Option<String> {
    let rest = hero_data_value.strip_prefix("file://{images}/")?;
    let stem = rest.rsplit_once('.').map(|(s, _)| s).unwrap_or(rest);
    if stem.contains("..") || stem.is_empty() {
        return None;
    }
    Some(format!("panorama/images/{stem}"))
}

/// The compiled file the game loads for a hero data image.
pub fn compiled_path(hero_data_value: &str) -> Option<String> {
    let ext = hero_data_value.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_else(|| "psd".into());
    image_stem(hero_data_value).map(|s| format!("{s}_{ext}.vtex_c"))
}

/// Scale a picture to cover `w`×`h` and crop the rest. Tall pictures in a
/// square or wide slot keep their top (where the face usually is).
pub fn fit(img: &image::DynamicImage, w: u32, h: u32) -> RgbaImage {
    let (sw, sh) = (img.width().max(1) as f64, img.height().max(1) as f64);
    let scale = (w as f64 / sw).max(h as f64 / sh);
    let (nw, nh) = (((sw * scale).round() as u32).max(w), ((sh * scale).round() as u32).max(h));
    let big = img.resize_exact(nw, nh, FilterType::Lanczos3).to_rgba8();
    let x = (nw - w) / 2;
    let src_ratio = sw / sh;
    let dst_ratio = w as f64 / h as f64;
    let y = if src_ratio < dst_ratio { ((nh - h) as f64 * 0.12).round() as u32 } else { (nh - h) / 2 };
    image::imageops::crop_imm(&big, x, y.min(nh - h), w, h).to_image()
}

/// A flat 8-bit RGBA Photoshop file (no layers): what the game's image
/// paths name, so the compiled file gets the name the game looks for.
pub fn write_psd(img: &RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let mut out = Vec::with_capacity((w * h * 4) as usize + 64);
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&1u16.to_be_bytes()); // version
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(&4u16.to_be_bytes()); // channels: R G B A
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&8u16.to_be_bytes()); // bits per channel
    out.extend_from_slice(&3u16.to_be_bytes()); // RGB
    out.extend_from_slice(&0u32.to_be_bytes()); // colour mode data
    out.extend_from_slice(&0u32.to_be_bytes()); // image resources
    out.extend_from_slice(&0u32.to_be_bytes()); // layers and masks
    out.extend_from_slice(&0u16.to_be_bytes()); // raw image data
    for c in 0..4 {
        out.extend(img.pixels().map(|p| p.0[c]));
    }
    out
}

/// The fitted picture as PNG (for the app's preview).
pub fn preview_png(source: &Path, field: &str) -> Result<Vec<u8>> {
    let (_, w, h) = slot(field).ok_or_else(|| Error::msg("Unknown image slot"))?;
    let img = image::open(source).map_err(|e| Error::msg(format!("Couldn't read {}: {e}", source.display())))?;
    let mut out = std::io::Cursor::new(Vec::new());
    fit(&img, w, h).write_to(&mut out, image::ImageFormat::Png).map_err(|e| Error::msg(e.to_string()))?;
    Ok(out.into_inner())
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct IconBuildReport {
    pub files: Vec<String>,
    pub vpk: String,
    pub vpk_size: u64,
    pub log: String,
}

/// Find what the compiler wrote for `stem` (its naming for loose images
/// may differ from the game's).
fn find_compiled(game_root: &Path, stem: &str) -> Option<PathBuf> {
    let p = stem.split('/').fold(game_root.to_path_buf(), |p, s| p.join(s));
    let dir = p.parent()?;
    let name = p.file_name()?.to_string_lossy().to_lowercase();
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|f| {
            let n = f.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            n.ends_with(".vtex_c") && (n == format!("{name}.vtex_c") || n.starts_with(&format!("{name}_")))
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Build a portrait mod. `images`: the hero's hero data image fields
/// (field, value). Pictures go in at the game's paths and compile to the
/// names the game loads.
#[allow(clippy::too_many_arguments)]
pub fn build(
    csdk_dir: &Path,
    slug: &str,
    images: &[(String, String)],
    jobs: &[IconJob],
    out_vpk: &Path,
    runner: &Runner,
    progress: &dyn Fn(&str),
) -> Result<IconBuildReport> {
    if jobs.is_empty() {
        return Err(Error::msg("Choose at least one picture"));
    }
    if !csdk::is_csdk_dir(csdk_dir) {
        return Err(Error::msg(format!("CSDK wasn't found at {} (game\\bin\\win64\\resourcecompiler.exe is missing)", csdk_dir.display())));
    }
    let addon = csdk::addon_name(slug);
    let content_root = csdk_dir.join("content").join("citadel_addons").join(&addon);
    let game_root = csdk_dir.join("game").join("citadel_addons").join(&addon);
    for d in [&content_root, &game_root] {
        if d.exists() {
            std::fs::remove_dir_all(d).at(d)?;
        }
    }
    let mut prepared = vec![]; // (stem, compiled path the game loads, psd file)
    for job in jobs {
        let (label, w, h) = slot(&job.field).ok_or_else(|| Error::msg(format!("Unknown image slot {}", job.field)))?;
        let value = images.iter().find(|(f, _)| f == &job.field).map(|(_, v)| v.clone()).ok_or_else(|| Error::msg(format!("This hero has no {label} image")))?;
        let stem = image_stem(&value).ok_or_else(|| Error::msg(format!("Unexpected image path {value}")))?;
        let target = compiled_path(&value).unwrap();
        progress(&format!("Preparing {label}"));
        let img = image::open(&job.source).map_err(|e| Error::msg(format!("Couldn't read {}: {e}", job.source)))?;
        let psd = stem.split('/').fold(content_root.clone(), |p, s| p.join(s)).with_extension("psd");
        std::fs::create_dir_all(psd.parent().unwrap()).at(&psd)?;
        std::fs::write(&psd, write_psd(&fit(&img, w, h))).at(&psd)?;
        prepared.push((label, stem, target, psd));
    }

    let mut log = String::new();
    for (label, stem, _, psd) in &prepared {
        progress(&format!("Compiling {label}"));
        let (_, out) = runner(csdk_dir, psd)?;
        log.push_str(&format!("== {stem}\n{}\n", out.trim()));
    }
    // Images the compiler wouldn't take one by one: compile them through a
    // panorama dynamic-images list, the way the game's own are built.
    if prepared.iter().any(|(_, stem, _, _)| find_compiled(&game_root, stem).is_none()) {
        progress("Compiling the images list");
        let list: String = prepared
            .iter()
            .map(|(_, stem, _, _)| format!("\t\t\"{{images}}/{}.psd\"\t\"\"\n", stem.trim_start_matches("panorama/images/")))
            .collect();
        let vpdi = content_root.join("panorama").join("dms_portraits.vpdi");
        std::fs::create_dir_all(vpdi.parent().unwrap()).at(&vpdi)?;
        std::fs::write(&vpdi, format!("\"DynamicImages\"\n{{\n\t\"Explicit Files\"\n\t{{\n{list}\t}}\n}}\n")).at(&vpdi)?;
        let (_, out) = runner(csdk_dir, &vpdi)?;
        log.push_str(&format!("== dynamic images list\n{}\n", out.trim()));
    }

    progress("Packing VPK");
    let mut packed: Vec<(String, Vec<u8>)> = vec![];
    for (label, stem, target, _) in &prepared {
        let f = find_compiled(&game_root, stem).ok_or_else(|| Error::msg(format!("CSDK couldn't compile the {label} image. Is Steam running?\n\n{}", log.trim())))?;
        packed.push((target.clone(), std::fs::read(&f).at(&f)?));
    }
    packed.sort_by(|a, b| a.0.cmp(&b.0));
    if let Some(p) = out_vpk.parent() {
        std::fs::create_dir_all(p).at(p)?;
    }
    vpk::write_vpk(out_vpk, &packed)?;
    Ok(IconBuildReport {
        files: packed.iter().map(|p| p.0.clone()).collect(),
        vpk: out_vpk.display().to_string(),
        vpk_size: std::fs::metadata(out_vpk).at(out_vpk)?.len(),
        log,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_paths() {
        assert_eq!(image_stem("file://{images}/heroes/unicorn_card.psd").as_deref(), Some("panorama/images/heroes/unicorn_card"));
        assert_eq!(compiled_path("file://{images}/heroes/unicorn_card_critical.psd").as_deref(), Some("panorama/images/heroes/unicorn_card_critical_psd.vtex_c"));
        assert_eq!(image_stem("file://{images}/../x.psd"), None);
    }

    #[test]
    fn fits_and_writes_psd() {
        let img = image::DynamicImage::ImageRgba8(RgbaImage::from_fn(560, 760, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 7, 200])));
        let card = fit(&img, 280, 380);
        assert_eq!(card.dimensions(), (280, 380));
        let sq = fit(&img, 128, 128);
        assert_eq!(sq.dimensions(), (128, 128));
        let psd = write_psd(&card);
        assert_eq!(&psd[..4], b"8BPS");
        assert_eq!(u32::from_be_bytes(psd[14..18].try_into().unwrap()), 380);
        assert_eq!(u32::from_be_bytes(psd[18..22].try_into().unwrap()), 280);
        assert_eq!(psd.len(), 26 + 4 + 4 + 4 + 2 + 280 * 380 * 4);
        assert_eq!(*psd.last().unwrap(), 200, "alpha kept");
    }

    #[test]
    fn builds_with_a_fake_compiler() {
        let t = tempfile::tempdir().unwrap();
        let csdk = t.path().join("csdk");
        std::fs::create_dir_all(csdk.join("game/bin/win64")).unwrap();
        std::fs::create_dir_all(csdk.join("content")).unwrap();
        std::fs::write(csdk::compiler_path(&csdk), b"").unwrap();
        let src = t.path().join("me.png");
        RgbaImage::from_pixel(560, 760, image::Rgba([1, 2, 3, 255])).save(&src).unwrap();
        let images = vec![("m_strIconHeroCard".to_string(), "file://{images}/heroes/unicorn_card.psd".to_string())];
        let jobs = vec![IconJob { field: "m_strIconHeroCard".into(), source: src.display().to_string() }];
        // Fake compiler: x.psd in content -> x_psd.vtex_c in game.
        let runner = |csdk: &Path, file: &Path| -> Result<(bool, String)> {
            let rel = file.strip_prefix(csdk.join("content")).unwrap();
            let out = csdk.join("game").join(rel).with_file_name(format!("{}_psd.vtex_c", file.file_stem().unwrap().to_string_lossy()));
            std::fs::create_dir_all(out.parent().unwrap()).unwrap();
            std::fs::write(out, b"vtex").unwrap();
            Ok((true, "OK".into()))
        };
        let out = t.path().join("p.vpk");
        let r = build(&csdk, "Mono icons", &images, &jobs, &out, &runner, &|_| {}).unwrap();
        assert_eq!(r.files, vec!["panorama/images/heroes/unicorn_card_psd.vtex_c"]);
        assert_eq!(vpk::list_files(&out).unwrap(), r.files);
    }
}
