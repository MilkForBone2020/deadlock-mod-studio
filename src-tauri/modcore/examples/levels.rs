//! Dev helper: measure audio files (raw audio or .vsnd_c).
fn main() {
    for a in std::env::args().skip(1) {
        let p = std::path::Path::new(&a);
        let bytes = std::fs::read(p).unwrap();
        let pcm = if a.ends_with(".vsnd_c") || a.ends_with(".bin") {
            let audio = if a.ends_with(".bin") { &bytes[..] } else { modcore::audio::vsnd_audio(&bytes).unwrap() };
            modcore::audio::decode(audio.to_vec(), modcore::audio::sniff_ext(audio))
        } else { modcore::audio::decode_file(p) };
        match pcm {
            Ok(pcm) => println!("{:?}  {}", modcore::audio::measure(&pcm), p.file_name().unwrap().to_string_lossy()),
            Err(e) => println!("ERR {e}  {a}"),
        }
    }
}
