//! Dev helper: dump sound events (name -> vsnd files) from .vsndevts_c files.
fn main() {
    for a in std::env::args().skip(1) {
        let kv = modcore::kv3::parse_resource(&std::fs::read(&a).unwrap());
        match kv {
            Ok(kv) => for (name, ev) in kv.as_object().unwrap_or(&[]) {
                let mut s = vec![]; if let Some(f) = ev.get("vsnd_files") { f.strings(&mut s); }
                let mut other = vec![]; for (k, v) in ev.as_object().unwrap_or(&[]) { if k.starts_with("vsnd_files_") { let mut x=vec![]; v.strings(&mut x); other.push(format!("{k}={}", x.len())); } }
                let base = ev.get("base").and_then(|b| b.as_str()).unwrap_or("");
                println!("{name}\tbase={base}\t{}\t{}", s.iter().map(|x| x.rsplit('/').next().unwrap()).collect::<Vec<_>>().join(","), other.join(" "));
            },
            Err(e) => eprintln!("ERR {a}: {e}"),
        }
    }
}
