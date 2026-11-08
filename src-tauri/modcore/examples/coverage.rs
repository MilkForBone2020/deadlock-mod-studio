//! Dev helper: per-hero coverage of the sound catalogue.
use std::collections::BTreeMap;
fn main() {
    let dir = modcore::vpk::read_directory(std::path::Path::new("/mnt/user-data/uploads/citadel/pak01_dir.vpk")).unwrap();
    let cat = modcore::catalog::Catalog::build(&dir, Some(std::path::Path::new("/mnt/user-data/uploads/citadel/resource/localization")));
    let hero_files: Vec<_> = dir.entries.iter().filter(|e| e.path.starts_with("soundevents/hero/")).map(|e| e.path.clone()).collect();
    let skipped_hero: Vec<_> = cat.skipped.iter().filter(|s| s.starts_with("soundevents/hero/")).collect();
    println!("heroes in data: {}  hero event files: {}  unreadable here (archive not copied): {}", cat.heroes.len(), hero_files.len(), skipped_hero.len());
    // per hero: per slot sound count, unmatched ability-like events
    let mut per: BTreeMap<String, ([usize; 5], usize, Vec<String>)> = BTreeMap::new();
    for (path, uses) in &cat.uses {
        if !path.starts_with("sounds/abilities/") { continue; }
        for u in uses {
            if u.hero.is_empty() || u.category == "Voice" { continue; }
            let e = per.entry(u.hero.clone()).or_default();
            match u.slot { Some(s) => e.0[s as usize] += 1, None => { e.1 += 1; if e.2.len() < 4 && !e.2.contains(&u.event) { e.2.push(u.event.clone()); } } }
        }
    }
    for h in &cat.heroes {
        let readable = !skipped_hero.iter().any(|s| s.ends_with(&format!("/{}.vsndevts_c", h.code)));
        let has_file = hero_files.iter().any(|f| f.ends_with(&format!("/{}.vsndevts_c", h.code)));
        let (slots, unmatched, ex) = per.get(&h.code).cloned().unwrap_or_default();
        let abil: Vec<String> = h.abilities.iter().map(|a| format!("{}:{}({}vo)", a.slot, a.name, a.voice_lines)).collect();
        println!("{:<14} {:<16} file={} read={} slots1-4={:?} unmatched={} {:?}\n    {}", h.code, h.name, has_file, readable, &slots[1..], unmatched, ex, abil.join(", "));
    }
}
