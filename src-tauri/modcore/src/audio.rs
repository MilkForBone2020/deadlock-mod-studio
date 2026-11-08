//! Audio: pulling the original sound out of a compiled `.vsnd_c`, decoding
//! any common format (MP3, WAV, MP4/M4A, OGG, FLAC), measuring loudness and
//! writing the WAV that Valve's compiler turns into a game sound.
//!
//! Ground rule: a replacement starts at the same loudness as the original
//! (turned up or down to match), and I can adjust it from there, but never to
//! more than `MAX_BOOST_DB` above the original. A limiter keeps boosted
//! sounds from clipping.

use crate::{Error, IoCtx, Result};
use serde::Serialize;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymErr;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Interleaved f32 samples.
#[derive(Clone, Debug)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }
    pub fn duration_secs(&self) -> f64 {
        self.frames() as f64 / self.rate.max(1) as f64
    }
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Level {
    /// Integrated loudness (LUFS), or RMS dBFS for very short sounds.
    pub loudness_db: f64,
    /// Sample peak in dBFS.
    pub peak_db: f64,
    pub duration_secs: f64,
    pub channels: u16,
    pub rate: u32,
}

/// The audio file stored inside a compiled sound (`.vsnd_c`): the first u32
/// is the size of Valve's header, and the original audio follows unchanged.
pub fn vsnd_audio(vsnd: &[u8]) -> Result<&[u8]> {
    if vsnd.len() < 16 {
        return Err(Error::msg("Compiled sound is too small"));
    }
    let header = u32::from_le_bytes([vsnd[0], vsnd[1], vsnd[2], vsnd[3]]) as usize;
    if header < 16 || header >= vsnd.len() {
        return Err(Error::msg("Compiled sound has no audio data"));
    }
    Ok(&vsnd[header..])
}

/// Guess a file extension for audio bytes, for the decoder's hint.
pub fn sniff_ext(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"RIFF") {
        "wav"
    } else if bytes.starts_with(b"OggS") {
        "ogg"
    } else if bytes.starts_with(b"fLaC") {
        "flac"
    } else if bytes.len() > 8 && &bytes[4..8] == b"ftyp" {
        "mp4"
    } else {
        "mp3"
    }
}

/// Decode audio bytes of any supported format to interleaved f32.
pub fn decode(bytes: Vec<u8>, ext: &str) -> Result<Pcm> {
    let mut hint = Hint::new();
    hint.with_extension(ext);
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), MediaSourceStreamOptions::default());
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| Error::msg(format!("Can't read this audio file: {e}")))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| Error::msg("The file has no audio track"))?
        .clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| Error::msg(format!("Unsupported audio codec: {e}")))?;
    let mut rate = track.codec_params.sample_rate.unwrap_or(0);
    let mut channels = track.codec_params.channels.map(|c| c.count() as u16).unwrap_or(0);
    let mut samples: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymErr::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymErr::ResetRequired) => break,
            Err(e) => return Err(Error::msg(format!("Audio read error: {e}"))),
        };
        if packet.track_id() != track.id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let spec = *buf.spec();
                rate = spec.rate;
                channels = spec.channels.count() as u16;
                let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
                sb.copy_interleaved_ref(buf);
                samples.extend_from_slice(sb.samples());
            }
            // A damaged frame: skip it, as players do.
            Err(SymErr::DecodeError(_)) => continue,
            Err(e) => return Err(Error::msg(format!("Audio decode error: {e}"))),
        }
    }
    if samples.is_empty() || rate == 0 || channels == 0 {
        return Err(Error::msg("The file decoded to no audio"));
    }
    Ok(Pcm { rate, channels, samples })
}

pub fn decode_file(path: &Path) -> Result<Pcm> {
    let bytes = std::fs::read(path).at(path)?;
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| sniff_ext(&bytes).to_string());
    let ext = if ext == "m4a" || ext == "mov" { "mp4".to_string() } else { ext };
    decode(bytes, &ext)
}

/// Decode the original audio inside a compiled sound.
pub fn decode_vsnd(vsnd: &[u8]) -> Result<Pcm> {
    let audio = vsnd_audio(vsnd)?;
    decode(audio.to_vec(), sniff_ext(audio))
}

fn db(x: f64) -> f64 {
    if x <= 0.0 {
        -120.0
    } else {
        20.0 * x.log10()
    }
}

pub fn measure(pcm: &Pcm) -> Level {
    let peak = pcm.samples.iter().fold(0f32, |m, s| m.max(s.abs())) as f64;
    let mut loud = f64::NEG_INFINITY;
    if pcm.duration_secs() >= 0.4 {
        if let Ok(mut m) = ebur128::EbuR128::new(pcm.channels as u32, pcm.rate, ebur128::Mode::I) {
            if m.add_frames_f32(&pcm.samples).is_ok() {
                loud = m.loudness_global().unwrap_or(f64::NEG_INFINITY);
            }
        }
    }
    if !loud.is_finite() {
        // Short sounds: fall back to RMS (close to LUFS for this purpose).
        let n = pcm.samples.len().max(1) as f64;
        let rms = (pcm.samples.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>() / n).sqrt();
        loud = db(rms);
    }
    Level {
        loudness_db: (loud * 10.0).round() / 10.0,
        peak_db: (db(peak) * 10.0).round() / 10.0,
        duration_secs: (pcm.duration_secs() * 100.0).round() / 100.0,
        channels: pcm.channels,
        rate: pcm.rate,
    }
}

/// The most a replacement may be louder than the original (integrated
/// loudness). Enforced here, whatever the UI sends.
pub const MAX_BOOST_DB: f64 = 3.0;
/// The quietest a user offset may go.
pub const MIN_OFFSET_DB: f64 = -24.0;
/// Sample peaks are held below this (dBFS) by the limiter.
pub const CEILING_DB: f64 = -0.5;

/// Gain (dB) that brings `new` to the original's loudness plus `offset_db`
/// (clamped to MIN_OFFSET_DB..=MAX_BOOST_DB).
pub fn plan_gain_db(original: &Level, new: &Level, offset_db: f64) -> f64 {
    let offset = offset_db.clamp(MIN_OFFSET_DB, MAX_BOOST_DB);
    let g = original.loudness_db + offset - new.loudness_db;
    (g * 10.0).round() / 10.0
}

/// Apply gain with a simple peak limiter (instant attack, 80 ms release)
/// holding samples under CEILING_DB. Returns the result and whether the
/// limiter had to act.
pub fn apply_gain(pcm: &Pcm, gain_db: f64) -> (Pcm, bool) {
    let gain = 10f64.powf(gain_db / 20.0) as f32;
    let ceiling = 10f64.powf(CEILING_DB / 20.0) as f32;
    let ch = pcm.channels.max(1) as usize;
    let release = (-1.0 / (0.08 * pcm.rate.max(1) as f64)).exp() as f32;
    let mut env = 0f32;
    let mut limited = false;
    let mut out = Vec::with_capacity(pcm.samples.len());
    for frame in pcm.samples.chunks(ch) {
        let peak = frame.iter().fold(0f32, |m, s| m.max((s * gain).abs()));
        env = peak.max(env * release);
        let g = if env > ceiling {
            limited = true;
            ceiling / env
        } else {
            1.0
        };
        out.extend(frame.iter().map(|s| (s * gain * g).clamp(-1.0, 1.0)));
    }
    (Pcm { rate: pcm.rate, channels: pcm.channels, samples: out }, limited)
}

/// Match `pcm` to the original's loudness plus a user offset, never more
/// than MAX_BOOST_DB above it. Returns the processed audio, the gain used,
/// whether the limiter acted, and the final level.
pub fn match_loudness(pcm: &Pcm, original: &Level, offset_db: f64) -> (Pcm, f64, bool, Level) {
    let yours = measure(pcm);
    let mut gain = plan_gain_db(original, &yours, offset_db);
    let (mut out, mut limited) = apply_gain(pcm, gain);
    let mut level = measure(&out);
    // Safety net: the limiter changes loudness a little; never exceed the cap.
    let cap = original.loudness_db + MAX_BOOST_DB + 0.1;
    if level.loudness_db > cap {
        gain -= level.loudness_db - cap + 0.1;
        let r = apply_gain(pcm, gain);
        out = r.0;
        limited = r.1;
        level = measure(&out);
    }
    (out, (gain * 10.0).round() / 10.0, limited, level)
}

/// Mix to the original's channel count (mono <-> stereo; other layouts are
/// averaged down to mono first).
pub fn to_channels(pcm: &Pcm, channels: u16) -> Pcm {
    if pcm.channels == channels || channels == 0 {
        return pcm.clone();
    }
    let src = pcm.channels as usize;
    let mono: Vec<f32> =
        pcm.samples.chunks(src).map(|f| f.iter().sum::<f32>() / src as f32).collect();
    let samples = if channels == 1 {
        mono
    } else {
        mono.iter().flat_map(|s| std::iter::repeat_n(*s, channels as usize)).collect()
    };
    Pcm { rate: pcm.rate, channels, samples }
}

/// Write 16-bit PCM WAV with a gain applied. With `looped`, add a `smpl`
/// chunk marking the whole file as a loop (and a matching `cue ` point), the
/// standard way to tell Valve's tools a sound loops.
pub fn write_wav(path: &Path, pcm: &Pcm, gain_db: f64, looped: bool) -> Result<()> {
    let gain = 10f64.powf(gain_db / 20.0) as f32;
    let mut data = Vec::with_capacity(pcm.samples.len() * 2);
    for s in &pcm.samples {
        let v = (s * gain).clamp(-1.0, 1.0);
        data.extend(((v * 32767.0).round() as i16).to_le_bytes());
    }
    let frames = pcm.frames() as u32;
    let block_align = pcm.channels * 2;
    let mut chunks: Vec<u8> = Vec::new();
    // fmt
    chunks.extend(b"fmt ");
    chunks.extend(16u32.to_le_bytes());
    chunks.extend(1u16.to_le_bytes());
    chunks.extend(pcm.channels.to_le_bytes());
    chunks.extend(pcm.rate.to_le_bytes());
    chunks.extend((pcm.rate * block_align as u32).to_le_bytes());
    chunks.extend(block_align.to_le_bytes());
    chunks.extend(16u16.to_le_bytes());
    if looped && frames > 1 {
        // cue: one point at sample 0
        chunks.extend(b"cue ");
        chunks.extend(28u32.to_le_bytes());
        chunks.extend(1u32.to_le_bytes());
        chunks.extend(1u32.to_le_bytes()); // id
        chunks.extend(0u32.to_le_bytes()); // position
        chunks.extend(b"data");
        chunks.extend(0u32.to_le_bytes()); // chunk start
        chunks.extend(0u32.to_le_bytes()); // block start
        chunks.extend(0u32.to_le_bytes()); // sample offset
        // smpl: one forward loop over the whole file
        chunks.extend(b"smpl");
        chunks.extend(60u32.to_le_bytes());
        for v in [0u32, 0, 1_000_000_000 / pcm.rate.max(1), 60, 0, 0, 0, 1, 0] {
            chunks.extend(v.to_le_bytes());
        }
        for v in [1u32, 0, 0, frames - 1, 0, 0] {
            chunks.extend(v.to_le_bytes());
        }
    }
    chunks.extend(b"data");
    chunks.extend((data.len() as u32).to_le_bytes());
    chunks.extend(&data);
    if data.len() % 2 == 1 {
        chunks.push(0);
    }
    let mut out = Vec::with_capacity(12 + chunks.len());
    out.extend(b"RIFF");
    out.extend(((4 + chunks.len()) as u32).to_le_bytes());
    out.extend(b"WAVE");
    out.extend(chunks);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }
    std::fs::write(path, out).at(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, channels: u16, secs: f64, amp: f32) -> Pcm {
        let n = (rate as f64 * secs) as usize;
        let mut samples = Vec::with_capacity(n * channels as usize);
        for i in 0..n {
            let v = amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin();
            for _ in 0..channels {
                samples.push(v);
            }
        }
        Pcm { rate, channels, samples }
    }

    #[test]
    fn wav_round_trip_and_loop_chunk() {
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("x.wav");
        let pcm = tone(44100, 2, 1.0, 0.5);
        write_wav(&p, &pcm, 0.0, true).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert!(bytes.windows(4).any(|w| w == b"smpl"));
        assert!(bytes.windows(4).any(|w| w == b"cue "));
        // hound must accept it as a valid WAV
        let r = hound::WavReader::open(&p).unwrap();
        assert_eq!(r.spec().channels, 2);
        assert_eq!(r.spec().sample_rate, 44100);
        let back = decode_file(&p).unwrap();
        assert_eq!(back.frames(), pcm.frames());
    }

    #[test]
    fn matches_loudness_both_ways() {
        let orig = measure(&tone(48000, 1, 1.0, 0.25));
        // Louder: turned down to match.
        let (_, g, _, l) = match_loudness(&tone(48000, 1, 1.0, 0.9), &orig, 0.0);
        assert!(g < -10.0 && g > -12.0, "gain {g}");
        assert!((l.loudness_db - orig.loudness_db).abs() < 0.3, "{l:?} vs {orig:?}");
        // Quieter: turned up to match.
        let (_, g, _, l) = match_loudness(&tone(48000, 1, 1.0, 0.05), &orig, 0.0);
        assert!(g > 13.0, "gain {g}");
        assert!((l.loudness_db - orig.loudness_db).abs() < 0.3, "{l:?} vs {orig:?}");
        // User offset down.
        let (_, _, _, l) = match_loudness(&tone(48000, 1, 1.0, 0.5), &orig, -6.0);
        assert!((l.loudness_db - (orig.loudness_db - 6.0)).abs() < 0.3);
    }

    #[test]
    fn boost_is_capped_and_limited() {
        // A loud original (peaks near full scale) and a request for +10 dB:
        // capped at +3 dB, limiter keeps peaks under the ceiling.
        let orig = measure(&tone(44100, 2, 1.0, 0.8));
        let (out, _, limited, l) = match_loudness(&tone(44100, 2, 1.0, 0.3), &orig, 10.0);
        assert!(l.loudness_db <= orig.loudness_db + MAX_BOOST_DB + 0.2, "{l:?} vs {orig:?}");
        assert!(limited);
        assert!(measure(&out).peak_db <= CEILING_DB + 0.05);
        assert_eq!(plan_gain_db(&orig, &orig, 99.0), MAX_BOOST_DB);
        assert_eq!(plan_gain_db(&orig, &orig, -99.0), MIN_OFFSET_DB);
    }

    #[test]
    fn short_sounds_use_rms() {
        let l = measure(&tone(44100, 1, 0.1, 0.5));
        assert!(l.loudness_db < -5.0 && l.loudness_db > -12.0, "{l:?}");
    }

    #[test]
    fn channel_conversion() {
        let st = tone(44100, 2, 0.1, 0.5);
        let mono = to_channels(&st, 1);
        assert_eq!(mono.channels, 1);
        assert_eq!(mono.frames(), st.frames());
        let back = to_channels(&mono, 2);
        assert_eq!(back.samples.len(), st.samples.len());
    }

    #[test]
    fn vsnd_header_split() {
        let mut v = vec![0u8; 40];
        v[0] = 32;
        v[32..].copy_from_slice(b"RIFFabcd");
        assert_eq!(vsnd_audio(&v).unwrap(), b"RIFFabcd");
        assert!(vsnd_audio(&[0u8; 8]).is_err());
    }

    /// Real files from Luke's PC, only present on the dev machine.
    #[test]
    fn real_compiled_sound_matches_its_source() {
        let v = Path::new("/mnt/user-data/uploads/Reduced_CSDK_13/game/citadel_addons/billy_audio_template/sounds/abilities/punkgoat/a3/punkgoat_blasted_lp.vsnd_c");
        let m = Path::new("/mnt/user-data/uploads/Reduced_CSDK_13/content/citadel_addons/billy_audio_template/sounds/abilities/punkgoat/a3/punkgoat_blasted_lp.mp3");
        if !v.exists() {
            return;
        }
        let vb = std::fs::read(v).unwrap();
        assert_eq!(vsnd_audio(&vb).unwrap(), std::fs::read(m).unwrap().as_slice());
        let lv = measure(&decode_vsnd(&vb).unwrap());
        let lm = measure(&decode_file(m).unwrap());
        assert_eq!(lv, lm);
        assert!(lv.duration_secs > 1.0);
    }
}
