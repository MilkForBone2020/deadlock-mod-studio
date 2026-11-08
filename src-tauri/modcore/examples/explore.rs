//! Dev helper: list entries in a VPK matching a filter.
//! cargo run --example explore -- <vpk> <filter>
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let d = modcore::vpk::read_directory(std::path::Path::new(&a[1])).unwrap();
    let f = a.get(2).cloned().unwrap_or_default();
    let mut n = 0;
    let total = d.entries.len();
    let sounds = d.entries.iter().filter(|e| e.path.ends_with(".vsnd_c")).count();
    for e in d.entries.iter().filter(|e| e.path.contains(&f)) {
        n += 1;
        if n <= 60 { println!("{}  arch={} off={} len={} pre={}", e.path, e.archive_index, e.offset, e.length, e.preload.len()); }
    }
    eprintln!("{n} matches; {total} entries; {sounds} .vsnd_c");
}
