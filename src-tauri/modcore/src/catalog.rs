//! Sound catalogue: which hero, ability and part of an ability each game
//! sound belongs to, worked out from the game's own data:
//!
//! - `soundevents/**/*.vsndevts_c`: event name -> sound files
//!   (e.g. `Gigawatt.StormCloud.Bolt.Cast` -> `..._storm_cloud_bolt_cast.vsnd`)
//! - `scripts/heroes.vdata_c`: each hero's abilities by slot
//! - `resource/localization/*_english.txt`: display names (Seven, Storm Cloud)

use crate::kv3::{self, Kv};
use crate::vpk::Directory;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SoundUse {
    /// Hero code name, e.g. `gigawatt`, or empty for shared sounds.
    pub hero: String,
    pub hero_name: String,
    /// Weapon, Ability, Movement, Melee, Voice, Item, UI, Music, Other
    pub category: String,
    /// Ability slot 1–4 (4 is the ultimate), when it belongs to one.
    pub slot: Option<u8>,
    pub ability_name: Option<String>,
    /// Which part of it, e.g. "Bolt cast", "Loop", "End".
    pub part: String,
    pub event: String,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AbilityInfo {
    pub slot: u8,
    pub key: String,
    pub name: String,
    /// Voice lines the hero says when using it (`<hero>_use_power<slot>_*`).
    pub voice_lines: usize,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HeroInfo {
    pub code: String,
    pub name: String,
    pub abilities: Vec<AbilityInfo>,
}

#[derive(Default, Debug)]
pub struct Catalog {
    /// `sounds/...vsnd_c` -> where it's used
    pub uses: HashMap<String, Vec<SoundUse>>,
    pub heroes: Vec<HeroInfo>,
    /// Sound event files that couldn't be read (shown as a note, not fatal).
    pub skipped: Vec<String>,
}

/// Parse `"key" "value"` pairs from a Valve localisation file.
pub fn parse_localization(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with("//") {
            continue;
        }
        let parts: Vec<&str> = line.split('"').collect();
        if parts.len() >= 5 && !parts[1].is_empty() {
            out.insert(parts[1].to_lowercase(), parts[3].to_string());
        }
    }
    out
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase()
}

/// `citadel_ability_storm_cloud` / `ability_power_surge` -> `stormcloud` / `powersurge`
fn ability_norm(key: &str, hero: &str) -> String {
    let k = key.to_lowercase();
    let k = k.strip_prefix("citadel_ability_").or_else(|| k.strip_prefix("ability_")).unwrap_or(&k);
    let k = k.strip_prefix(&format!("{hero}_")).unwrap_or(k);
    norm(k)
}

/// Turn event-name pieces into words: ["Bolt","Aoe","Lp"] -> "Bolt AoE loop"
pub fn humanize(parts: &[&str]) -> String {
    let mut words: Vec<String> = Vec::new();
    for p in parts {
        // split CamelCase
        let mut cur = String::new();
        for (i, ch) in p.chars().enumerate() {
            if i > 0 && ch.is_ascii_uppercase() && cur.chars().last().is_some_and(|c| c.is_ascii_lowercase()) {
                words.push(std::mem::take(&mut cur));
            }
            cur.push(ch);
        }
        if !cur.is_empty() {
            words.push(cur);
        }
    }
    let mapped: Vec<String> = words
        .iter()
        .map(|w| match w.to_lowercase().as_str() {
            _ if w.chars().any(|c| c.is_ascii_digit()) => w.clone(),
            "lp" | "loop" => "loop".into(),
            "proj" => "projectile".into(),
            "explo" => "explosion".into(),
            "aoe" => "AoE".into(),
            "wpn" => "weapon".into(),
            "vo" => "voice".into(),
            "sfx" => "effect".into(),
            _ => w.to_lowercase(),
        })
        .collect();
    let s = mapped.join(" ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn category_for(event: &str, file_dir: &str) -> String {
    let e = event.to_lowercase();
    let has = |k: &str| e.split('.').any(|p| p == k) || e.contains(k);
    if file_dir.starts_with("soundevents/vo") || has(".vo") || e.ends_with("vo") {
        "Voice".into()
    } else if has("wpn") || has("weapon") || has("whizby") || has("bullet") || has("reload") || has("zoom") {
        "Weapon".into()
    } else if has("melee") {
        "Melee".into()
    } else if has("footstep") || has("movement") || has("mantle") || has("jumpland") || has("ladderstep") || has("slide") || has("dash") {
        "Movement".into()
    } else if file_dir.contains("item") {
        "Item".into()
    } else if file_dir.contains("music") || has("music") {
        "Music".into()
    } else if file_dir.contains("ui") {
        "UI".into()
    } else {
        "Other".into()
    }
}

/// Every sound file an event plays (`vsnd_files`, `vsnd_files_tail`, ...).
fn event_files(ev: &Kv) -> Vec<String> {
    let mut out = Vec::new();
    for (k, v) in ev.as_object().unwrap_or(&[]) {
        if k.starts_with("vsnd_files") {
            let mut s = Vec::new();
            v.strings(&mut s);
            for f in s {
                let f = f.to_lowercase().replace('\\', "/");
                if f.ends_with(".vsnd") {
                    out.push(format!("{f}_c"));
                } else if f.ends_with(".vsnd_c") {
                    out.push(f);
                }
            }
        }
    }
    out
}

impl Catalog {
    /// Build from the game's pak01 directory and its localisation folder.
    /// Unreadable files are skipped, never fatal: worst case the picker
    /// falls back to file names.
    pub fn build(dir: &Directory, localization_dir: Option<&Path>) -> Catalog {
        let mut cat = Catalog::default();
        let mut loc = HashMap::new();
        if let Some(l) = localization_dir {
            for f in [
                "citadel_gc_hero_names/citadel_gc_hero_names_english.txt",
                "citadel_heroes/citadel_heroes_english.txt",
                "citadel_main/citadel_main_english.txt",
            ] {
                if let Ok(t) = std::fs::read_to_string(l.join(f)) {
                    loc.extend(parse_localization(&t));
                }
            }
        }
        let loc_get = |k: &str| -> Option<String> {
            let k = k.to_lowercase();
            loc.get(&k).or_else(|| loc.get(&format!("{k}:n"))).cloned()
        };

        // Heroes and their ability slots.
        let mut heroes: BTreeMap<String, HeroInfo> = BTreeMap::new();
        if let Some(e) = dir.find("scripts/heroes.vdata_c") {
            match dir.extract(e).ok().and_then(|b| kv3::parse_resource(&b).ok()) {
                Some(kv) => {
                    for (key, h) in kv.as_object().unwrap_or(&[]) {
                        let Some(code) = key.strip_prefix("hero_") else { continue };
                        let Some(bound) = h.get("m_mapBoundAbilities") else { continue };
                        let mut abilities = Vec::new();
                        for slot in 1..=4u8 {
                            if let Some(ak) = bound.get(&format!("ESlot_Signature_{slot}")).and_then(|v| v.as_str()) {
                                let name = loc_get(ak).unwrap_or_else(|| humanize(&[&ability_norm(ak, code)]));
                                abilities.push(AbilityInfo { slot, key: ak.to_string(), name, voice_lines: 0 });
                            }
                        }
                        let name = loc_get(&format!("hero_{code}")).unwrap_or_else(|| humanize(&[code]));
                        heroes.insert(code.to_string(), HeroInfo { code: code.to_string(), name, abilities });
                    }
                }
                None => cat.skipped.push("scripts/heroes.vdata_c".into()),
            }
        }

        // Voice line counts per ability.
        for h in heroes.values_mut() {
            for a in &mut h.abilities {
                a.voice_lines = voice_lines(dir, &h.code, a.slot).len();
            }
        }

        // Ability data: every sound event an ability (or its modifiers) names.
        // event name (lowercase) -> (hero, slot)
        let mut event_owner: HashMap<String, (String, u8)> = HashMap::new();
        if let Some(e) = dir.find("scripts/abilities.vdata_c") {
            match dir.extract(e).ok().and_then(|b| kv3::parse_resource(&b).ok()) {
                Some(kv) => {
                    for h in heroes.values() {
                        for a in &h.abilities {
                            if let Some(node) = kv.get(&a.key) {
                                let mut strs = Vec::new();
                                node.strings(&mut strs);
                                for st in strs {
                                    if st.contains('.') && !st.contains('/') && !st.contains(' ') {
                                        event_owner.entry(st.to_lowercase()).or_insert((h.code.clone(), a.slot));
                                    }
                                }
                            }
                        }
                    }
                }
                None => cat.skipped.push("scripts/abilities.vdata_c".into()),
            }
        }

        // Sound events.
        let files: Vec<_> = dir
            .entries
            .iter()
            .filter(|e| e.path.starts_with("soundevents/") && e.path.ends_with(".vsndevts_c"))
            .collect();
        for f in files {
            let Ok(bytes) = dir.extract(f) else {
                cat.skipped.push(f.path.clone());
                continue;
            };
            let Ok(kv) = kv3::parse_resource(&bytes) else {
                cat.skipped.push(f.path.clone());
                continue;
            };
            let file_dir = f.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            let stem = f.path.rsplit('/').next().unwrap_or("").trim_end_matches(".vsndevts_c");
            // Hero for this file: soundevents/hero/<code>, generated_vo_hero_<code>
            let file_hero = if file_dir.ends_with("/hero") {
                Some(stem.to_string())
            } else {
                stem.strip_prefix("generated_vo_hero_").map(|s| s.to_string())
            }
            .filter(|c| heroes.contains_key(c));

            for (event, ev) in kv.as_object().unwrap_or(&[]) {
                let paths = event_files(ev);
                if paths.is_empty() {
                    continue;
                }
                let parts: Vec<&str> = event.split('.').collect();
                // Hero: from the file, else from the event's first segment.
                let hero_code = file_hero.clone().or_else(|| {
                    parts.first().map(|p| p.to_lowercase()).filter(|p| heroes.contains_key(p))
                });
                let hero = hero_code.as_ref().and_then(|c| heroes.get(c));
                // The ability data is authoritative; it can also name the hero.
                let owner = event_owner.get(&event.to_lowercase()).cloned();
                let hero = owner.as_ref().and_then(|(c, _)| heroes.get(c)).or(hero);
                let base = ev.get("base").and_then(|b| b.as_str()).unwrap_or("");
                let (slot, drop) = match (&owner, hero) {
                    (Some((_, s)), Some(h)) => (Some(*s), match_ability(&parts, h).map(|m| m.1).unwrap_or_default()),
                    (None, Some(h)) => match match_ability(&parts, h) {
                        Some((s, d)) => (Some(s), d),
                        None if base.contains(".Ult") && h.abilities.iter().any(|a| a.slot == 4) => (Some(4), vec![]),
                        None => (folder_slot(&paths), vec![]),
                    },
                    _ => (None, vec![]),
                };
                let ability_name = slot.and_then(|s| hero.and_then(|h| h.abilities.iter().find(|a| a.slot == s)).map(|a| a.name.clone()));
                let category = if slot.is_some() { "Ability".to_string() } else { category_for(event, file_dir) };
                // Part: the event name minus the hero prefix, slot markers and
                // the ability's own name.
                let rest: Vec<&str> = parts
                    .iter()
                    .enumerate()
                    .filter(|(i, p)| *i > 0 && !drop.contains(i) && slot_marker(p).is_none())
                    .map(|(_, p)| *p)
                    .collect();
                let part = if rest.is_empty() { "Main".to_string() } else { humanize(&rest) };
                let u = SoundUse {
                    hero: hero.map(|h| h.code.clone()).unwrap_or_default(),
                    hero_name: hero.map(|h| h.name.clone()).unwrap_or_default(),
                    category,
                    slot,
                    ability_name,
                    part,
                    event: event.clone(),
                };
                for p in paths {
                    let mut u = u.clone();
                    // Voice lines belong to the hero speaking them: sounds/vo/<hero>/
                    if let Some(speaker) = p.strip_prefix("sounds/vo/").and_then(|r| r.split('/').next()) {
                        if let Some(h) = heroes.get(speaker) {
                            u.category = "Voice".into();
                            u.hero = h.code.clone();
                            u.hero_name = h.name.clone();
                            if u.part.is_empty() || u.slot.is_none() {
                                u.part = voice_part(event, speaker);
                            }
                            if u.slot.is_none() {
                                u.slot = use_power_slot(&p, speaker);
                                u.ability_name = u.slot.and_then(|s| h.abilities.iter().find(|a| a.slot == s)).map(|a| a.name.clone());
                            }
                        }
                    }
                    let list = cat.uses.entry(p).or_default();
                    if !list.contains(&u) {
                        list.push(u);
                    }
                }
            }
        }
        // Only heroes that actually have sounds (hides test and unreleased
        // entries like "targetdummy").
        let with_sounds: std::collections::HashSet<String> =
            cat.uses.values().flatten().map(|u| u.hero.clone()).filter(|h| !h.is_empty()).collect();
        cat.heroes = heroes.into_values().filter(|h| with_sounds.contains(&h.code)).collect();
        cat
    }

    pub fn uses_of(&self, path: &str) -> &[SoundUse] {
        self.uses.get(path).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

/// `A1`..`A4` in an event name marks the ability slot directly.
fn slot_marker(part: &str) -> Option<u8> {
    let p = part.as_bytes();
    if p.len() == 2 && (p[0] == b'A' || p[0] == b'a') && (b'1'..=b'4').contains(&p[1]) {
        Some(p[1] - b'0')
    } else {
        None
    }
}

/// Words too generic to identify an ability on their own.
const STOP: &[&str] = &[
    "cast", "hit", "impact", "start", "end", "stop", "loop", "lp", "proj", "projectile", "modifier", "mod", "explo",
    "explode", "explosion", "detonate", "proc", "lp2", "fire", "wpn", "whizby", "victim", "target", "self", "other",
    "enemy", "ally", "charge", "channel", "activate", "deactivate", "ready", "select", "unselect", "buff", "debuff",
    "damage", "aoe", "duration", "dur", "tick", "warning", "alert", "confirm", "begin", "exit", "enter", "shoot",
];

/// Find which of the hero's abilities an event name refers to. Returns the
/// slot and which name parts spelled it (to leave out of the "part" label).
fn match_ability(parts: &[&str], h: &HeroInfo) -> Option<(u8, Vec<usize>)> {
    for (i, p) in parts.iter().enumerate() {
        if let Some(s) = slot_marker(p) {
            if h.abilities.iter().any(|a| a.slot == s) {
                return Some((s, vec![i]));
            }
        }
    }
    let names: Vec<(u8, String, String)> =
        h.abilities.iter().map(|a| (a.slot, ability_norm(&a.key, &h.code), norm(&a.name))).collect();
    // Exact match on 1–3 consecutive parts (e.g. Wrecking + Ball).
    for len in (1..=3).rev() {
        for start in 1..parts.len() {
            if start + len > parts.len() {
                break;
            }
            let joined = norm(&parts[start..start + len].concat());
            if joined.len() < 3 {
                continue;
            }
            if let Some((s, _, _)) = names.iter().find(|(_, k, n)| &joined == k || &joined == n) {
                return Some((*s, (start..start + len).collect()));
            }
        }
    }
    // A distinctive word contained in the ability's name (Flask in
    // Alchemical Flask, Grenade in Consecrating Grenade).
    for (i, p) in parts.iter().enumerate().skip(1) {
        let pn = norm(p);
        if pn.len() < 4 || STOP.contains(&pn.as_str()) {
            continue;
        }
        let hits: Vec<_> = names.iter().filter(|(_, k, n)| k.contains(&pn) || n.contains(&pn)).collect();
        if hits.len() == 1 {
            return Some((hits[0].0, vec![i]));
        }
    }
    None
}

/// Seven-style folders: `sounds/abilities/<hero>/a4_storm_cloud/...`
fn folder_slot(paths: &[String]) -> Option<u8> {
    paths.iter().find_map(|p| {
        p.split('/').find_map(|seg| {
            let b = seg.as_bytes();
            (b.len() > 2 && b[0] == b'a' && (b'1'..=b'4').contains(&b[1]) && b[2] == b'_').then(|| b[1] - b'0')
        })
    })
}

/// `necro_ping_careful_gigawatt_02_ping_2d` -> "Ping careful gigawatt"
fn voice_part(event: &str, speaker: &str) -> String {
    let e = event.to_lowercase();
    let mut e = e.strip_prefix(&format!("{speaker}_")).unwrap_or(&e).to_string();
    for suffix in ["_hero_zipline_3d", "_hero_3d", "_ping_2d", "_2d", "_3d"] {
        if let Some(x) = e.strip_suffix(suffix) {
            e = x.to_string();
            break;
        }
    }
    let words: Vec<&str> = e
        .split(['_', '.'])
        .filter(|w| !w.is_empty() && !w.chars().all(|c| c.is_ascii_digit()) && !matches!(*w, "2d" | "3d" | "hero" | "vo"))
        .collect();
    let s = words.join(" ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => "Voice line".into(),
    }
}

/// `sounds/vo/gigawatt/gigawatt_use_power4_02.vsnd_c` -> Some(4)
fn use_power_slot(path: &str, hero: &str) -> Option<u8> {
    let name = path.rsplit('/').next()?;
    if !name.starts_with(hero) {
        return None;
    }
    let i = name.find("_use_power")?;
    let d = name[i + "_use_power".len()..].chars().next()?.to_digit(10)? as u8;
    (1..=4).contains(&d).then_some(d)
}

/// Voice lines a hero says when using ability `slot`, e.g.
/// `sounds/vo/gigawatt/gigawatt_use_power4_03.vsnd_c` and the
/// `..._use_power4_as_enemy_..` lines heard when they are on the other team.
pub fn voice_lines(dir: &Directory, hero: &str, slot: u8) -> Vec<String> {
    let folder = format!("sounds/vo/{hero}/");
    let tag = format!("_use_power{slot}_");
    dir.entries
        .iter()
        .filter(|e| {
            e.path.starts_with(&folder)
                && e.path.ends_with(".vsnd_c")
                && !e.path[folder.len()..].contains('/')
                && e.path.rsplit('/').next().is_some_and(|n| n.starts_with(hero) && n.contains(&tag))
        })
        .map(|e| e.path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanize_words() {
        assert_eq!(humanize(&["Bolt", "Aoe", "Lp"]), "Bolt AoE loop");
        assert_eq!(humanize(&["Proj", "Explo"]), "Projectile explosion");
        assert_eq!(humanize(&["ChainMod", "Victim", "Impact"]), "Chain mod victim impact");
        assert_eq!(humanize(&["Cast"]), "Cast");
        assert_eq!(humanize(&["L2R"]), "L2R");
        assert_eq!(voice_part("necro_ping_careful_gigawatt_02_ping_2d", "necro"), "Ping careful gigawatt");
        assert_eq!(use_power_slot("sounds/vo/gigawatt/gigawatt_use_power4_02.vsnd_c", "gigawatt"), Some(4));
        assert_eq!(use_power_slot("sounds/vo/gigawatt/gigawatt_happy_02.vsnd_c", "gigawatt"), None);
        assert_eq!(use_power_slot("sounds/vo/magician/magician_henry_use_power3_01.vsnd_c", "magician"), Some(3));
        assert_eq!(slot_marker("A3"), Some(3));
        assert_eq!(slot_marker("A5"), None);
        assert_eq!(folder_slot(&["sounds/abilities/gigawatt/a4_storm_cloud/end.vsnd_c".into()]), Some(4));
        let h = HeroInfo {
            code: "warden".into(),
            name: "Warden".into(),
            abilities: vec![
                AbilityInfo { slot: 1, key: "citadel_ability_warden_flask".into(), name: "Alchemical Flask".into(), voice_lines: 0 },
                AbilityInfo { slot: 4, key: "citadel_ability_last_stand".into(), name: "Last Stand".into(), voice_lines: 0 },
            ],
        };
        assert_eq!(match_ability(&["Warden", "Flask", "Cast"], &h).map(|m| m.0), Some(1));
        assert_eq!(match_ability(&["Warden", "Last", "Stand", "Lp"], &h), Some((4, vec![1, 2])));
        assert_eq!(match_ability(&["Warden", "Cast"], &h), None);
    }

    #[test]
    fn localization_pairs() {
        let t = "\"lang\"\n{\n\t\"Tokens\"\n\t{\n\t\t\"hero_gigawatt:n\" \"Seven\"\n\t\t// \"x\" \"y\"\n\t\t\"citadel_ability_storm_cloud\"\t\t\"Storm Cloud\"\n\t}\n}\n";
        let m = parse_localization(t);
        assert_eq!(m.get("hero_gigawatt:n").unwrap(), "Seven");
        assert_eq!(m.get("citadel_ability_storm_cloud").unwrap(), "Storm Cloud");
        assert!(!m.contains_key("x"));
    }

    #[test]
    fn ability_names_normalise() {
        assert_eq!(ability_norm("citadel_ability_storm_cloud", "gigawatt"), "stormcloud");
        assert_eq!(ability_norm("ability_power_surge", "gigawatt"), "powersurge");
        assert_eq!(norm("StormCloud"), "stormcloud");
    }

    /// Real game data from Luke's PC (dev machine only).
    #[test]
    fn real_catalog_for_seven() {
        let pak = Path::new("/mnt/user-data/uploads/citadel/pak01_dir.vpk");
        if !pak.exists() || !Path::new("/mnt/user-data/uploads/citadel/pak01_107.vpk").exists() {
            return;
        }
        let dir = crate::vpk::read_directory(pak).unwrap();
        let cat = Catalog::build(&dir, Some(Path::new("/mnt/user-data/uploads/citadel/resource/localization")));
        let seven = cat.heroes.iter().find(|h| h.code == "gigawatt").expect("Seven");
        assert_eq!(seven.name, "Seven");
        let names: Vec<(u8, &str)> = seven.abilities.iter().map(|a| (a.slot, a.name.as_str())).collect();
        eprintln!("{names:?} skipped {}", cat.skipped.len());
        assert_eq!(names[3].0, 4);
        assert!(seven.abilities[3].voice_lines >= 10);
        let bolt = cat.uses_of("sounds/abilities/gigawatt/gigawatt_storm_cloud_bolt_cast.vsnd_c");
        assert_eq!(bolt.len(), 1);
        assert_eq!(bolt[0].slot, Some(4));
        assert_eq!(bolt[0].part, "Bolt cast");
        assert_eq!(bolt[0].hero_name, "Seven");
        let lp = cat.uses_of("sounds/abilities/gigawatt/gigawatt_storm_cloud_lp.vsnd_c");
        assert_eq!(lp[0].part, "Loop");
        let ball = cat.uses_of("sounds/abilities/gigawatt/gigawatt_lightning_ball_cast.vsnd_c");
        assert_eq!(ball[0].slot, Some(1));
        let fire = cat.uses_of("sounds/weapons/gigawatt/gigawatt_weapon_fire_01.vsnd_c");
        eprintln!("fire: {fire:?}");
        for (p, u) in cat.uses.iter().filter(|(p, _)| p.contains("gigawatt")).take(12) {
            eprintln!("{p}: {}", u.iter().map(|x| format!("{} / {:?} {:?} / {} ({})", x.category, x.slot, x.ability_name, x.part, x.event)).collect::<Vec<_>>().join(" | "));
        }
    }
}
