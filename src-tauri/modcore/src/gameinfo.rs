//! Reading and patching `game/citadel/gameinfo.gi`.
//!
//! Ground rule: the app only ever touches the `SearchPaths` block, and only
//! to add or remove its own line: `Game citadel/addons_studio`. Other lines,
//! including the `Game citadel/addons` line used by Deadlock Mod Manager,
//! ConVars and everything else, are left byte-for-byte as they are.

use crate::{Error, Result};

pub const ADDONS_VALUE: &str = "citadel/addons_studio";
/// The folder other mod managers use. Never added or removed by this app.
pub const SHARED_ADDONS_VALUE: &str = "citadel/addons";

/// Strip a `//` comment that is not inside quotes.
fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_quote = !in_quote,
            b'/' if !in_quote && i + 1 < bytes.len() && bytes[i + 1] == b'/' => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

/// Split a KeyValues line into tokens, honouring quotes and treating braces
/// as their own tokens.
fn tokens(line: &str) -> Vec<String> {
    let line = strip_comment(line);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for c in line.chars() {
        if in_quote {
            if c == '"' {
                in_quote = false;
                out.push(std::mem::take(&mut cur));
            } else {
                cur.push(c);
            }
            continue;
        }
        match c {
            '"' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                in_quote = true;
            }
            '{' | '}' => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                out.push(c.to_string());
            }
            c if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Split text into lines, keeping each line's ending so the file can be
/// rebuilt exactly (Valve ships CRLF on Windows).
fn split_keep_endings(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            out.push(&text[start..=i]);
            start = i + 1;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

fn body(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}

/// Find the `SearchPaths { ... }` block. Returns (index of the line holding
/// the opening brace, index of the line holding the closing brace).
fn find_search_paths(lines: &[&str]) -> Result<(usize, usize)> {
    let mut i = 0;
    while i < lines.len() {
        let t = tokens(body(lines[i]));
        if t.first().map(|s| s.eq_ignore_ascii_case("SearchPaths")) == Some(true) {
            // The `{` may be on this line or a following one.
            let mut open = None;
            if t.get(1).map(|s| s == "{") == Some(true) {
                open = Some(i);
            } else {
                let mut j = i + 1;
                while j < lines.len() {
                    let tj = tokens(body(lines[j]));
                    if tj.is_empty() {
                        j += 1;
                        continue;
                    }
                    if tj[0] == "{" {
                        open = Some(j);
                    }
                    break;
                }
            }
            let open = open.ok_or_else(|| Error::msg("SearchPaths has no opening brace"))?;
            let mut depth = 0i32;
            for (k, line) in lines.iter().enumerate().skip(open) {
                for tok in tokens(body(line)) {
                    if tok == "{" {
                        depth += 1;
                    } else if tok == "}" {
                        depth -= 1;
                        if depth == 0 {
                            return Ok((open, k));
                        }
                    }
                }
            }
            return Err(Error::msg("SearchPaths block is never closed"));
        }
        i += 1;
    }
    Err(Error::msg("Could not find the SearchPaths block in gameinfo.gi"))
}

fn is_addons_line(line: &str) -> bool {
    let t = tokens(body(line));
    t.len() == 2 && t[0].eq_ignore_ascii_case("Game") && t[1].eq_ignore_ascii_case(ADDONS_VALUE)
}

/// Our line goes directly above the first `Game citadel/addons` (another
/// manager's folder) or `Game citadel` line, so my own mods take priority
/// over both.
fn is_insert_anchor(line: &str) -> bool {
    let t = tokens(body(line));
    t.len() == 2
        && t[0].eq_ignore_ascii_case("Game")
        && (t[1].eq_ignore_ascii_case("citadel") || t[1].eq_ignore_ascii_case(SHARED_ADDONS_VALUE))
}

/// True when another manager's `Game citadel/addons` line is present.
pub fn has_shared_addons_path(text: &str) -> Result<bool> {
    let lines = split_keep_endings(text);
    let (open, close) = find_search_paths(&lines)?;
    Ok(lines[open..=close].iter().any(|l| {
        let t = tokens(body(l));
        t.len() == 2 && t[0].eq_ignore_ascii_case("Game") && t[1].eq_ignore_ascii_case(SHARED_ADDONS_VALUE)
    }))
}

/// Marks the `Mod` / `Write` lines this app adds, so only those are removed.
const MARK: &str = "// deadlock mod studio";

fn key_of(line: &str) -> Option<String> {
    let t = tokens(body(line));
    (t.len() == 2).then(|| t[0].to_ascii_lowercase())
}

fn is_our_extra(line: &str) -> bool {
    body(line).trim_end().ends_with(MARK) && matches!(key_of(line).as_deref(), Some("mod") | Some("write"))
}

/// True when the SearchPaths block already loads the addons folder and
/// keeps `citadel` as the game's MOD and write folder. (Without `Mod` and
/// `Write` lines the game makes the FIRST `Game` folder its MOD folder, our
/// mods folder, and then fails to find its own settings: "Unable to read
/// default keybinding configuration".)
pub fn has_addons_path(text: &str) -> Result<bool> {
    let lines = split_keep_endings(text);
    let (open, close) = find_search_paths(&lines)?;
    let block = &lines[open..=close];
    let has = |k: &str| block.iter().any(|l| key_of(l).as_deref() == Some(k));
    Ok(block.iter().any(|l| is_addons_line(l)) && has("mod") && has("write"))
}

/// True when any line this app adds is in SearchPaths (to clean up, even
/// from an install patched by an older version).
pub fn has_any_own_line(text: &str) -> Result<bool> {
    let lines = split_keep_endings(text);
    let (open, close) = find_search_paths(&lines)?;
    Ok(lines[open..=close].iter().any(|l| is_addons_line(l) || is_our_extra(l)))
}

/// Add `Game citadel/addons_studio` directly above the first `Game
/// citadel/addons` or `Game citadel` line in SearchPaths, copying that line's
/// indentation and spacing. Returns the text
/// unchanged if the line is already there.
pub fn add_addons_path(text: &str) -> Result<String> {
    if has_addons_path(text)? {
        return Ok(text.to_string());
    }
    let lines = split_keep_endings(text);
    let (open, close) = find_search_paths(&lines)?;
    let target = (open..=close)
        .find(|&i| is_insert_anchor(lines[i]) || is_addons_line(lines[i]))
        .ok_or_else(|| Error::msg("SearchPaths has no `Game citadel` line to insert above"))?;
    let have_game = lines[open..=close].iter().any(|l| is_addons_line(l));
    let have_mod = lines[open..=close].iter().any(|l| key_of(l).as_deref() == Some("mod"));
    let have_write = lines[open..=close].iter().any(|l| key_of(l).as_deref() == Some("write"));

    let model = lines[target];
    let content = body(model);
    let ending = &model[content.len()..];
    let ending = if ending.is_empty() { "\n" } else { ending };
    let indent_len = content.len() - content.trim_start().len();
    let indent = &content[..indent_len];
    let rest = &content[indent_len..];
    // Key exactly as written (`Game` or `"Game"`), then the spacing before
    // its value, both copied from the model line.
    let key_len = if rest.starts_with('"') { 6 } else { 4 };
    let key = &rest[..key_len];
    let after_key = &rest[key_len..];
    let gap_len = after_key.len() - after_key.trim_start().len();
    let gap = if gap_len == 0 { "\t" } else { &after_key[..gap_len] };
    let quoted = after_key.trim_start().starts_with('"');
    let q = |v: &str| if quoted { format!("\"{v}\"") } else { v.to_string() };
    let kq = |k: &str| if key.starts_with('"') { format!("\"{k}\"") } else { k.to_string() };
    let new_line = if have_game { String::new() } else { format!("{indent}{key}{gap}{}{ending}", q(ADDONS_VALUE)) };
    // Keep citadel as the game's MOD and write folder (see has_addons_path),
    // at the end of the list as Valve lays them out in Counter-Strike 2.
    let mut extra = String::new();
    if !have_mod {
        extra.push_str(&format!("{indent}{}{gap}{}\t{MARK}{ending}", kq("Mod"), q("citadel")));
    }
    if !have_write {
        extra.push_str(&format!("{indent}{}{gap}{}\t{MARK}{ending}", kq("Write"), q("citadel")));
    }
    // The closing brace may share its line with the last entry: then the
    // extras go before that line's brace-only part isn't possible, so
    // before the closing line.
    let mut out = String::with_capacity(text.len() + new_line.len() + extra.len());
    for (i, l) in lines.iter().enumerate() {
        if i == target {
            out.push_str(&new_line);
        }
        if i == close {
            out.push_str(&extra);
        }
        out.push_str(l);
    }
    validate(&out)?;
    Ok(out)
}

/// Remove every `Game citadel/addons_studio` line from SearchPaths. Text outside
/// that block is never changed.
pub fn remove_addons_path(text: &str) -> Result<String> {
    let lines = split_keep_endings(text);
    let (open, close) = find_search_paths(&lines)?;
    let out: String = lines
        .iter()
        .enumerate()
        .filter(|(i, l)| !(*i >= open && *i <= close && (is_addons_line(l) || is_our_extra(l))))
        .map(|(_, l)| *l)
        .collect();
    validate(&out)?;
    Ok(out)
}

/// Basic sanity check: braces balance and quotes close. Catches the
/// "Failed to parse KeyValues" class of mistakes before the game sees them.
pub fn validate(text: &str) -> Result<()> {
    let mut depth = 0i64;
    for (n, line) in text.lines().enumerate() {
        let stripped = strip_comment(line);
        if !stripped.matches('"').count().is_multiple_of(2) {
            return Err(Error::msg(format!("Unclosed quote on line {}", n + 1)));
        }
        for tok in tokens(line) {
            if tok == "{" {
                depth += 1;
            } else if tok == "}" {
                depth -= 1;
                if depth < 0 {
                    return Err(Error::msg(format!("Unexpected `}}` on line {}", n + 1)));
                }
            }
        }
    }
    if depth != 0 {
        return Err(Error::msg("Braces in gameinfo.gi do not balance"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\"GameInfo\"\r\n{\r\n\tgame \"citadel\"\r\n\tFileSystem\r\n\t{\r\n\t\t//\r\n\t\t// comment about search paths\r\n\t\t//\r\n\t\tSearchPaths\r\n\t\t{\r\n\t\t\tMod                 citadel\r\n\t\t\tWrite               citadel\r\n\t\t\tGame                citadel\r\n\t\t\tWrite               core\r\n\t\t\tMod                 core\r\n\t\t\tGame                core\r\n\t\t}\r\n\t}\r\n\tConVars\r\n\t{\r\n\t\t\"r_shadows\" \"1\" // keep\r\n\t}\r\n}\r\n";

    #[test]
    fn keeps_citadel_as_the_mod_and_write_folder() {
        let gi = "\"GameInfo\"\r\n{\r\n\tFileSystem\r\n\t{\r\n\t\tSearchPaths\r\n\t\t{\r\n\t\t\tGame_LowViolence\tcitadel_lv\r\n\r\n\t\t\tGame\t\t\t\tcitadel\r\n\t\t\tGame\t\t\t\tcore\r\n\t\t}\r\n\t}\r\n}\r\n";
        let out = add_addons_path(gi).unwrap();
        assert!(out.contains("\t\t\tGame\t\t\t\tcitadel/addons_studio\r\n\t\t\tGame\t\t\t\tcitadel\r\n\t\t\tGame\t\t\t\tcore\r\n\t\t\tMod\t\t\t\tcitadel\t// deadlock mod studio\r\n\t\t\tWrite\t\t\t\tcitadel\t// deadlock mod studio\r\n\t\t}"), "{out}");
        assert!(has_addons_path(&out).unwrap());
        assert_eq!(add_addons_path(&out).unwrap(), out, "idempotent");
        assert_eq!(remove_addons_path(&out).unwrap(), gi, "restored exactly");
        // An install patched by an older version (only the Game line) gets the two lines added.
        let old = gi.replace("\t\t\tGame\t\t\t\tcitadel\r\n", "\t\t\tGame\t\t\t\tcitadel/addons_studio\r\n\t\t\tGame\t\t\t\tcitadel\r\n");
        assert!(!has_addons_path(&old).unwrap());
        let fixed = add_addons_path(&old).unwrap();
        assert_eq!(fixed, out);
        // A Mod line Valve ships is never removed.
        let valve = gi.replace("\t\t\tGame\t\t\t\tcore\r\n", "\t\t\tGame\t\t\t\tcore\r\n\t\t\tMod\t\t\t\tcitadel\r\n");
        let v2 = add_addons_path(&valve).unwrap();
        assert_eq!(v2.matches("Mod\t").count(), 1);
        assert_eq!(remove_addons_path(&v2).unwrap(), valve);
    }

    #[test]
    fn detects_missing_line() {
        assert!(!has_addons_path(SAMPLE).unwrap());
    }

    #[test]
    fn adds_line_above_game_citadel_with_same_spacing() {
        let out = add_addons_path(SAMPLE).unwrap();
        assert!(has_addons_path(&out).unwrap());
        assert!(out.contains(
            "\t\t\tWrite               citadel\r\n\t\t\tGame                citadel/addons_studio\r\n\t\t\tGame                citadel\r\n"
        ));
        assert_eq!(out.len(), SAMPLE.len() + "\t\t\tGame                citadel/addons_studio\r\n".len());
    }

    #[test]
    fn add_is_idempotent() {
        let once = add_addons_path(SAMPLE).unwrap();
        let twice = add_addons_path(&once).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn remove_restores_exact_original() {
        let added = add_addons_path(SAMPLE).unwrap();
        let removed = remove_addons_path(&added).unwrap();
        assert_eq!(removed, SAMPLE);
    }

    #[test]
    fn convars_untouched() {
        let out = add_addons_path(SAMPLE).unwrap();
        assert!(out.contains("\t\t\"r_shadows\" \"1\" // keep\r\n"));
    }

    #[test]
    fn handles_brace_on_same_line_and_quotes_and_lf() {
        let t = "GameInfo\n{\n  FileSystem\n  {\n    SearchPaths {\n      \"Game\" \"citadel\"\n      \"Game\" \"core\"\n    }\n  }\n}\n";
        let out = add_addons_path(t).unwrap();
        assert!(out.contains("      \"Game\" \"citadel/addons_studio\"\n      \"Game\" \"citadel\"\n"));
        assert_eq!(remove_addons_path(&out).unwrap(), t);
    }

    #[test]
    fn ignores_addons_line_outside_search_paths() {
        let t = "GameInfo\n{\n  Game citadel/addons_studio\n  SearchPaths\n  {\n    Game citadel\n  }\n}\n";
        assert!(!has_addons_path(t).unwrap());
        let removed = remove_addons_path(t).unwrap();
        assert_eq!(removed, t);
    }

    #[test]
    fn validate_catches_bad_braces() {
        assert!(validate("a\n{\n").is_err());
        assert!(validate("a\n}\n").is_err());
        assert!(validate("a \"b\n").is_err());
        assert!(validate(SAMPLE).is_ok());
    }

    #[test]
    fn coexists_with_other_manager_line() {
        let t = "GameInfo\n{\n  SearchPaths\n  {\n    Game\tcitadel/addons\n    Game\tcitadel\n  }\n}\n";
        assert!(!has_addons_path(t).unwrap());
        assert!(has_shared_addons_path(t).unwrap());
        let out = add_addons_path(t).unwrap();
        // Ours goes above theirs, theirs is untouched.
        assert!(out.contains("    Game\tcitadel/addons_studio\n    Game\tcitadel/addons\n    Game\tcitadel\n"));
        let back = remove_addons_path(&out).unwrap();
        assert_eq!(back, t, "removing ours must leave the other manager's line");
        assert!(has_shared_addons_path(&back).unwrap());
    }

    #[test]
    fn missing_search_paths_errors() {
        assert!(add_addons_path("GameInfo\n{\n}\n").is_err());
    }
}
