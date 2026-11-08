//! Building sound mods with CSDK's resource compiler.
//!
//! For each replacement: read the original from the game, decode my file,
//! match channels and volume (never louder), write a WAV at the original's
//! path inside a CSDK addon, compile it with `resourcecompiler.exe -i`, then
//! pack the compiled `.vsnd_c` files into one VPK.

use crate::audio::{self, Level};
use crate::{steam, vpk, Error, IoCtx, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Same settings as the sound template that ships with CSDK 13.
pub const ENCODING_TXT: &str = "<!-- kv3 encoding:text:version{e21c7f3c-8a33-41c5-9977-a76d3a32aa0d} format:generic:version{7412167c-06e9-4698-aff2-e63eb59037e7} -->\n{\n\tcompress = \n\t{\n\t\tformat = \"mp3\"\n\t\tminbitrate = 128\n\t\tmaxbitrate = 320\n\t\tvbr = 1\n\t}\n}\n";

/// Addon folders this app creates inside CSDK all start with this, and the
/// app only ever deletes folders with this prefix.
pub const ADDON_PREFIX: &str = "dms_";

pub fn compiler_path(csdk: &Path) -> PathBuf {
    csdk.join("game").join("bin").join("win64").join("resourcecompiler.exe")
}

pub fn is_csdk_dir(csdk: &Path) -> bool {
    compiler_path(csdk).is_file() && csdk.join("content").is_dir()
}

pub fn detect() -> Option<PathBuf> {
    let mut c = vec![];
    for drive in ["C:", "D:", "E:"] {
        for base in ["", "\\Desktop", "\\Modding", "\\Games", "\\Downloads"] {
            c.push(PathBuf::from(format!("{drive}{base}\\Reduced_CSDK_13")));
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let h = PathBuf::from(home);
        c.push(h.join("Desktop").join("Reduced_CSDK_13"));
        c.push(h.join("Downloads").join("Reduced_CSDK_13"));
    }
    c.into_iter().find(|p| is_csdk_dir(p))
}

/// `sounds/a/b.vsnd_c` -> `sounds/a/b`
fn target_stem(target: &str) -> Result<String> {
    let t = target.to_lowercase().replace('\\', "/");
    let stem = t
        .strip_suffix(".vsnd_c")
        .ok_or_else(|| Error::msg(format!("{target} is not a compiled sound (.vsnd_c)")))?;
    if !stem.starts_with("sounds/") || stem.contains("..") {
        return Err(Error::msg(format!("{target} is not under sounds/")));
    }
    Ok(stem.to_string())
}

/// Loops are named `..._lp` or `..._loop` in Deadlock.
pub fn is_loop(target: &str) -> bool {
    let name = target.rsplit('/').next().unwrap_or(target).to_lowercase();
    let stem = name.trim_end_matches(".vsnd_c");
    stem.ends_with("_lp") || stem.contains("_lp_") || stem.ends_with("_loop") || stem.ends_with("_lp1")
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SoundJob {
    /// Game path of the sound replaced, e.g. `sounds/abilities/x.vsnd_c`.
    pub target: String,
    /// My replacement file (MP3, WAV, MP4, M4A, OGG, FLAC).
    pub source: String,
    /// Volume relative to the original's loudness, in dB. 0 = same as the
    /// original; capped at audio::MAX_BOOST_DB.
    #[serde(default)]
    pub volume_db: f64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ItemReport {
    pub target: String,
    pub original: Level,
    pub yours: Level,
    /// The level after processing (what plays in game).
    pub result: Level,
    /// Gain applied to my file.
    pub gain_db: f64,
    /// Requested volume relative to the original (after the cap).
    pub volume_db: f64,
    pub limited: bool,
    pub looped: bool,
    pub converted_channels: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BuildReport {
    pub items: Vec<ItemReport>,
    pub vpk: String,
    pub vpk_size: u64,
    pub log: String,
    /// Voice lines replaced with silence.
    pub muted: usize,
}

/// Runs the compiler on one source file. Returns (success, output).
pub type Runner<'a> = dyn Fn(&Path, &Path) -> Result<(bool, String)> + 'a;

/// The real runner: `resourcecompiler.exe -nop4 -f -i <file>`, hidden window.
pub fn run_compiler(csdk: &Path, file: &Path) -> Result<(bool, String)> {
    let exe = compiler_path(csdk);
    let mut cmd = std::process::Command::new(&exe);
    cmd.current_dir(exe.parent().unwrap_or(csdk))
        .arg("-nop4")
        .arg("-f")
        .arg("-i")
        .arg(file);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd.output().at(&exe)?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // The output file is checked by the caller; this catches outright errors.
    let ok = out.status.success() || text.contains(", 0 failed");
    Ok((ok, text))
}

/// Safe name for a CSDK addon folder: lowercase, a-z0-9_ only.
pub fn addon_name(slug: &str) -> String {
    let s: String = slug
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{ADDON_PREFIX}{}", s.trim_matches('_'))
}

/// Build one VPK from `jobs`. `game_dir` is Deadlock's install (for the
/// originals), `out_vpk` where the packed mod is written.
#[allow(clippy::too_many_arguments)]
pub fn build_sound_mod(
    csdk: &Path,
    game_dir: &Path,
    slug: &str,
    jobs: &[SoundJob],
    mutes: &[String],
    out_vpk: &Path,
    runner: &Runner,
    progress: &dyn Fn(&str),
) -> Result<BuildReport> {
    if jobs.is_empty() && mutes.is_empty() {
        return Err(Error::msg("Add at least one sound before building"));
    }
    if !is_csdk_dir(csdk) {
        return Err(Error::msg(format!(
            "CSDK wasn't found at {} (game\\bin\\win64\\resourcecompiler.exe is missing)",
            csdk.display()
        )));
    }
    let addon = addon_name(slug);
    let content_root = csdk.join("content").join("citadel_addons").join(&addon);
    let game_root = csdk.join("game").join("citadel_addons").join(&addon);
    for d in [&content_root, &game_root] {
        if d.exists() {
            std::fs::remove_dir_all(d).at(d)?;
        }
    }
    std::fs::create_dir_all(content_root.join("sounds")).at(&content_root)?;
    std::fs::write(content_root.join("sounds").join("encoding.txt"), ENCODING_TXT)
        .at(&content_root)?;

    let pak = steam::citadel_dir(game_dir).join("pak01_dir.vpk");
    let dir = vpk::read_directory(&pak)?;
    let mut items = Vec::new();
    let mut log = String::new();
    let mut packed: Vec<(String, Vec<u8>)> = Vec::new();

    for (n, job) in jobs.iter().enumerate() {
        let stem = target_stem(&job.target)?;
        let short = stem.rsplit('/').next().unwrap_or(&stem).to_string();
        progress(&format!("Preparing {short} ({}/{})", n + 1, jobs.len()));
        let entry = dir.find(&job.target).ok_or_else(|| {
            Error::msg(format!("{} isn't in the game's files (was it renamed in an update?)", job.target))
        })?;
        let original_pcm = audio::decode_vsnd(&dir.extract(entry)?)?;
        let original = audio::measure(&original_pcm);

        let src = Path::new(&job.source);
        if !src.is_file() {
            return Err(Error::msg(format!("Your file for {short} is missing: {}", src.display())));
        }
        let mut pcm = audio::decode_file(src)
            .map_err(|e| Error::msg(format!("{}: {e}", src.display())))?;
        let converted_channels = pcm.channels != original.channels;
        pcm = audio::to_channels(&pcm, original.channels);
        let yours = audio::measure(&pcm);
        let volume_db = job.volume_db.clamp(audio::MIN_OFFSET_DB, audio::MAX_BOOST_DB);
        let (processed, gain_db, limited, result) = audio::match_loudness(&pcm, &original, volume_db);
        let looped = is_loop(&job.target);
        let wav = content_root.join(format!("{stem}.wav"));
        audio::write_wav(&wav, &processed, 0.0, looped)?;

        progress(&format!("Compiling {short} ({}/{})", n + 1, jobs.len()));
        let (ok, out) = runner(csdk, &wav)?;
        log.push_str(&format!("== {stem}\n{}\n", out.trim()));
        let compiled = game_root.join(format!("{stem}.vsnd_c"));
        if !ok || !compiled.is_file() {
            return Err(Error::msg(format!(
                "CSDK couldn't compile {short}. Is Steam running?\n\n{}",
                out.trim()
            )));
        }
        packed.push((format!("{stem}.vsnd_c"), std::fs::read(&compiled).at(&compiled)?));
        items.push(ItemReport {
            target: job.target.clone(),
            original,
            yours,
            result,
            gain_db,
            volume_db,
            limited,
            looped,
            converted_channels,
        });
    }

    // Muted voice lines: a short silence at each line's path.
    let silence = audio::Pcm { rate: 44100, channels: 1, samples: vec![0.0; 4410] };
    for (n, target) in mutes.iter().enumerate() {
        let stem = target_stem(target)?;
        progress(&format!("Muting voice lines ({}/{})", n + 1, mutes.len()));
        let wav = content_root.join(format!("{stem}.wav"));
        audio::write_wav(&wav, &silence, 0.0, false)?;
        let (ok, out) = runner(csdk, &wav)?;
        log.push_str(&format!("== {stem} (muted)\n{}\n", out.trim()));
        let compiled = game_root.join(format!("{stem}.vsnd_c"));
        if !ok || !compiled.is_file() {
            return Err(Error::msg(format!("CSDK couldn't compile the muted line {stem}.\n\n{}", out.trim())));
        }
        packed.push((format!("{stem}.vsnd_c"), std::fs::read(&compiled).at(&compiled)?));
    }

    progress("Packing VPK");
    if let Some(p) = out_vpk.parent() {
        std::fs::create_dir_all(p).at(p)?;
    }
    vpk::write_vpk(out_vpk, &packed)?;
    let vpk_size = std::fs::metadata(out_vpk).at(out_vpk)?.len();
    Ok(BuildReport { items, vpk: out_vpk.display().to_string(), vpk_size, log, muted: mutes.len() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake compiler: wraps the WAV in a minimal vsnd_c-shaped header and
    /// writes it where the real one would.
    fn fake_runner(csdk: &Path, file: &Path) -> Result<(bool, String)> {
        let content = csdk.join("content");
        let rel = file.strip_prefix(&content).unwrap();
        let out = csdk.join("game").join(rel).with_extension("vsnd_c");
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        let wav = std::fs::read(file).unwrap();
        let mut v = vec![0u8; 32];
        v[0] = 32;
        v.extend(wav);
        std::fs::write(&out, v).unwrap();
        Ok((true, " OK: 1 compiled, 0 failed, 0 skipped".into()))
    }

    fn fake_vsnd_from_pcm(pcm: &audio::Pcm) -> Vec<u8> {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("o.wav");
        audio::write_wav(&p, pcm, 0.0, false).unwrap();
        let mut v = vec![0u8; 32];
        v[0] = 32;
        v.extend(std::fs::read(&p).unwrap());
        v
    }

    fn tone(rate: u32, ch: u16, secs: f64, amp: f32) -> audio::Pcm {
        let n = (rate as f64 * secs) as usize;
        let mut s = Vec::new();
        for i in 0..n {
            let v = amp * (2.0 * std::f32::consts::PI * 300.0 * i as f32 / rate as f32).sin();
            for _ in 0..ch {
                s.push(v);
            }
        }
        audio::Pcm { rate, channels: ch, samples: s }
    }

    #[test]
    fn builds_a_sound_mod_end_to_end() {
        let t = tempfile::tempdir().unwrap();
        // Fake CSDK
        let csdk = t.path().join("csdk");
        std::fs::create_dir_all(csdk.join("content")).unwrap();
        std::fs::create_dir_all(compiler_path(&csdk).parent().unwrap()).unwrap();
        std::fs::write(compiler_path(&csdk), b"").unwrap();
        // Fake game with two originals
        let game = t.path().join("steamapps/common/Deadlock");
        std::fs::create_dir_all(steam::citadel_dir(&game)).unwrap();
        std::fs::write(steam::gameinfo_path(&game), "x").unwrap();
        let orig_lp = fake_vsnd_from_pcm(&tone(44100, 2, 1.0, 0.2));
        let orig_hit = fake_vsnd_from_pcm(&tone(44100, 1, 0.5, 0.5));
        let pak = vpk::build_vpk(&[
            ("sounds/abilities/gigawatt/gigawatt_storm_cloud_lp.vsnd_c".into(), orig_lp),
            ("sounds/abilities/gigawatt/a4_storm_cloud/impact_1.vsnd_c".into(), orig_hit),
        ])
        .unwrap();
        std::fs::write(steam::citadel_dir(&game).join("pak01_dir.vpk"), pak).unwrap();
        // My files: a loud mono loop and a quiet stereo hit
        let mine_lp = t.path().join("my loop.wav");
        audio::write_wav(&mine_lp, &tone(48000, 1, 2.0, 0.9), 0.0, false).unwrap();
        let mine_hit = t.path().join("hit.wav");
        audio::write_wav(&mine_hit, &tone(44100, 2, 0.3, 0.1), 0.0, false).unwrap();

        let jobs = vec![
            SoundJob { target: "sounds/abilities/gigawatt/gigawatt_storm_cloud_lp.vsnd_c".into(), source: mine_lp.display().to_string(), volume_db: 0.0 },
            SoundJob { target: "sounds/abilities/gigawatt/a4_storm_cloud/impact_1.vsnd_c".into(), source: mine_hit.display().to_string(), volume_db: 2.0 },
        ];
        let mutes = vec!["sounds/vo/gigawatt/gigawatt_use_power4_01.vsnd_c".to_string()];
        let out = t.path().join("out/mod.vpk");
        let r = build_sound_mod(&csdk, &game, "Storm Cloud remix!", &jobs, &mutes, &out, &fake_runner, &|_| {}).unwrap();

        assert_eq!(r.items.len(), 2);
        assert_eq!(r.muted, 1);
        let lp = &r.items[0];
        assert!(lp.looped && lp.converted_channels);
        assert!(lp.gain_db < -10.0, "loud loop must be turned down: {}", lp.gain_db);
        let hit = &r.items[1];
        assert!(hit.gain_db > 0.0, "quiet hit is turned up: {}", hit.gain_db);
        assert!((hit.result.loudness_db - (hit.original.loudness_db + 2.0)).abs() < 0.5, "{:?}", hit);
        assert!(!hit.looped);

        let files = vpk::list_files(&out).unwrap();
        assert_eq!(files, vec![
            "sounds/abilities/gigawatt/a4_storm_cloud/impact_1.vsnd_c",
            "sounds/abilities/gigawatt/gigawatt_storm_cloud_lp.vsnd_c",
            "sounds/vo/gigawatt/gigawatt_use_power4_01.vsnd_c",
        ]);
        // The packed loop, decoded, is no louder than the original.
        let d = vpk::read_directory(&out).unwrap();
        let packed = d.extract(d.find("sounds/abilities/gigawatt/gigawatt_storm_cloud_lp.vsnd_c").unwrap()).unwrap();
        let after = audio::measure(&audio::decode_vsnd(&packed).unwrap());
        assert!(after.loudness_db <= lp.original.loudness_db + 0.3, "{after:?} vs {:?}", lp.original);
        assert_eq!(after.channels, 2);
        // Addon folder uses our prefix and a safe name
        assert!(csdk.join("content/citadel_addons/dms_storm_cloud_remix/sounds/encoding.txt").exists());
    }

    #[test]
    fn rejects_bad_targets_and_missing_csdk() {
        assert!(target_stem("models/x.vmdl_c").is_err());
        assert!(target_stem("sounds/../x.vsnd_c").is_err());
        assert!(target_stem("sounds/a/b.vsnd_c").is_ok());
        let t = tempfile::tempdir().unwrap();
        let jobs = vec![SoundJob { target: "sounds/a.vsnd_c".into(), source: "x".into(), volume_db: 0.0 }];
        let e = build_sound_mod(t.path(), t.path(), "x", &jobs, &[], &t.path().join("o.vpk"), &fake_runner, &|_| {});
        assert!(e.unwrap_err().to_string().contains("CSDK wasn't found"));
    }

    #[test]
    fn loop_names() {
        assert!(is_loop("sounds/abilities/gigawatt/gigawatt_storm_cloud_lp.vsnd_c"));
        assert!(is_loop("sounds/x/proj_lp.vsnd_c"));
        assert!(!is_loop("sounds/x/impact_1.vsnd_c"));
        assert!(!is_loop("sounds/x/help.vsnd_c"));
    }
}
