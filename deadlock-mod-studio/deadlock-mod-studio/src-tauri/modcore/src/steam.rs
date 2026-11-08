//! Finding Deadlock and reading its Steam build ID.

use std::path::{Path, PathBuf};

pub const APP_ID: &str = "1422450";
pub const DEFAULT_GAME_DIR: &str = r"D:\Games\SteamLibrary\steamapps\common\Deadlock";

/// `<game>/game/citadel`
pub fn citadel_dir(game_dir: &Path) -> PathBuf {
    game_dir.join("game").join("citadel")
}

pub fn gameinfo_path(game_dir: &Path) -> PathBuf {
    citadel_dir(game_dir).join("gameinfo.gi")
}

pub fn addons_dir(game_dir: &Path) -> PathBuf {
    citadel_dir(game_dir).join("addons")
}

/// A folder counts as a Deadlock install if it has game/citadel/gameinfo.gi.
pub fn is_game_dir(game_dir: &Path) -> bool {
    gameinfo_path(game_dir).is_file()
}

/// `.../steamapps/common/Deadlock` -> `.../steamapps/appmanifest_1422450.acf`
pub fn manifest_path(game_dir: &Path) -> Option<PathBuf> {
    let steamapps = game_dir.parent()?.parent()?;
    Some(steamapps.join(format!("appmanifest_{APP_ID}.acf")))
}

/// Read a top-level `"key" "value"` pair from a Steam .acf / .vdf file.
pub fn acf_value(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let parts: Vec<&str> = line.split('"').collect();
        // `\t"buildid"\t\t"123"` splits into ["\t", "buildid", "\t\t", "123", ""]
        if parts.len() >= 5 && parts[1].eq_ignore_ascii_case(key) {
            return Some(parts[3].to_string());
        }
    }
    None
}

/// Current Steam build ID of the installed game, if the manifest is readable.
pub fn build_id(game_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(manifest_path(game_dir)?).ok()?;
    acf_value(&text, "buildid")
}

/// Every `"path"` entry in Steam's libraryfolders.vdf.
pub fn library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('"').collect();
            if parts.len() >= 5 && parts[1].eq_ignore_ascii_case("path") {
                Some(PathBuf::from(parts[3].replace("\\\\", "\\")))
            } else {
                None
            }
        })
        .collect()
}

/// Look for Deadlock: the configured default first, then every Steam library.
pub fn detect_game_dir() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from(DEFAULT_GAME_DIR)];
    let steam_roots = [
        r"C:\Program Files (x86)\Steam",
        r"C:\Program Files\Steam",
    ];
    for root in steam_roots {
        let root = PathBuf::from(root);
        candidates.push(root.join("steamapps").join("common").join("Deadlock"));
        let vdf = root.join("steamapps").join("libraryfolders.vdf");
        if let Ok(text) = std::fs::read_to_string(&vdf) {
            for lib in library_paths(&text) {
                candidates.push(lib.join("steamapps").join("common").join("Deadlock"));
            }
        }
    }
    candidates.into_iter().find(|c| is_game_dir(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_buildid() {
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"1422450\"\n\t\"name\"\t\t\"Deadlock\"\n\t\"buildid\"\t\t\"20512345\"\n}\n";
        assert_eq!(acf_value(acf, "buildid").as_deref(), Some("20512345"));
        assert_eq!(acf_value(acf, "missing"), None);
    }

    #[test]
    fn reads_library_paths() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\Games\\\\SteamLibrary\"\n\t}\n}\n";
        let libs = library_paths(vdf);
        assert_eq!(libs.len(), 2);
        assert_eq!(libs[1], PathBuf::from(r"D:\Games\SteamLibrary"));
    }

    #[test]
    fn manifest_sits_in_steamapps() {
        let g = PathBuf::from("/lib/steamapps/common/Deadlock");
        assert_eq!(
            manifest_path(&g).unwrap(),
            PathBuf::from("/lib/steamapps/appmanifest_1422450.acf")
        );
    }
}
