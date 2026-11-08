//! Minimal read-only VPK (v1 / v2) directory reader.
//!
//! Only the directory tree is read: enough to check a file really is a VPK
//! and to list the internal paths it overrides (for conflict hints and the
//! "what's inside" view). File contents are never extracted.

use crate::{Error, IoCtx, Result};
use std::io::Read;
use std::path::Path;

pub const SIGNATURE: u32 = 0x55AA_1234;

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(Error::msg("VPK directory ends early (file is damaged or not a VPK)"));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn cstr(&mut self) -> Result<String> {
        let rest = &self.data[self.pos..];
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| Error::msg("VPK directory string is not terminated"))?;
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.pos += end + 1;
        Ok(s)
    }
}

/// Quick check of the 4-byte signature.
pub fn is_vpk(path: &Path) -> Result<bool> {
    let mut f = std::fs::File::open(path).at(path)?;
    let mut buf = [0u8; 4];
    if f.read(&mut buf).at(path)? < 4 {
        return Ok(false);
    }
    Ok(u32::from_le_bytes(buf) == SIGNATURE)
}

/// List every internal file path in a VPK, e.g. `sounds/abilities/x.vsnd_c`.
pub fn list_files(path: &Path) -> Result<Vec<String>> {
    let data = std::fs::read(path).at(path)?;
    list_files_from_bytes(&data)
}

pub fn list_files_from_bytes(data: &[u8]) -> Result<Vec<String>> {
    let mut c = Cursor { data, pos: 0 };
    if c.u32()? != SIGNATURE {
        return Err(Error::msg("Not a VPK file (wrong signature)"));
    }
    let version = c.u32()?;
    let tree_size = c.u32()? as usize;
    match version {
        1 => {}
        2 => {
            c.take(16)?; // file data, archive md5, other md5, signature sizes
        }
        v => return Err(Error::msg(format!("Unsupported VPK version {v}"))),
    }
    let tree_end = c.pos + tree_size;
    if tree_end > data.len() {
        return Err(Error::msg("VPK directory is larger than the file"));
    }

    let mut files = Vec::new();
    loop {
        let ext = c.cstr()?;
        if ext.is_empty() {
            break;
        }
        loop {
            let dir = c.cstr()?;
            if dir.is_empty() {
                break;
            }
            loop {
                let name = c.cstr()?;
                if name.is_empty() {
                    break;
                }
                let _crc = c.u32()?;
                let preload = c.u16()? as usize;
                let _archive = c.u16()?;
                let _offset = c.u32()?;
                let _length = c.u32()?;
                let terminator = c.u16()?;
                if terminator != 0xFFFF {
                    return Err(Error::msg("VPK entry is malformed"));
                }
                c.take(preload)?;

                let mut full = String::new();
                if dir != " " {
                    full.push_str(&dir);
                    full.push('/');
                }
                full.push_str(&name);
                if ext != " " {
                    full.push('.');
                    full.push_str(&ext);
                }
                files.push(full.to_lowercase());
            }
        }
        if c.pos > tree_end {
            return Err(Error::msg("VPK directory overruns its declared size"));
        }
    }
    files.sort();
    Ok(files)
}

/// Build a tiny v2 VPK directory in memory. Used by tests here and in sync.
#[doc(hidden)]
pub fn build_test_vpk(paths: &[&str]) -> Vec<u8> {
    use std::collections::BTreeMap;
    let mut tree: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for p in paths {
        let (dir, file) = match p.rfind('/') {
            Some(i) => (p[..i].to_string(), &p[i + 1..]),
            None => (" ".to_string(), *p),
        };
        let (name, ext) = match file.rfind('.') {
            Some(i) => (file[..i].to_string(), file[i + 1..].to_string()),
            None => (file.to_string(), " ".to_string()),
        };
        tree.entry(ext).or_default().entry(dir).or_default().push(name);
    }
    let mut t = Vec::new();
    for (ext, dirs) in tree {
        t.extend(ext.as_bytes());
        t.push(0);
        for (dir, names) in dirs {
            t.extend(dir.as_bytes());
            t.push(0);
            for n in names {
                t.extend(n.as_bytes());
                t.push(0);
                t.extend(0u32.to_le_bytes()); // crc
                t.extend(0u16.to_le_bytes()); // preload
                t.extend(0x7FFFu16.to_le_bytes()); // archive index (inline)
                t.extend(0u32.to_le_bytes());
                t.extend(0u32.to_le_bytes());
                t.extend(0xFFFFu16.to_le_bytes());
            }
            t.push(0);
        }
        t.push(0);
    }
    t.push(0);
    let mut out = Vec::new();
    out.extend(SIGNATURE.to_le_bytes());
    out.extend(2u32.to_le_bytes());
    out.extend((t.len() as u32).to_le_bytes());
    out.extend([0u8; 16]);
    out.extend(t);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_paths() {
        let bytes = build_test_vpk(&[
            "sounds/abilities/gigawatt/storm_cloud_cast.vsnd_c",
            "sounds/abilities/gigawatt/storm_cloud_lp.vsnd_c",
            "readme",
            "models/heroes/x.vmdl_c",
        ]);
        let files = list_files_from_bytes(&bytes).unwrap();
        assert_eq!(
            files,
            vec![
                "models/heroes/x.vmdl_c",
                "readme",
                "sounds/abilities/gigawatt/storm_cloud_cast.vsnd_c",
                "sounds/abilities/gigawatt/storm_cloud_lp.vsnd_c",
            ]
        );
    }

    #[test]
    fn rejects_non_vpk() {
        assert!(list_files_from_bytes(b"PK\x03\x04hello").is_err());
        assert!(list_files_from_bytes(&[0x34, 0x12, 0xAA, 0x55, 9, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn rejects_truncated() {
        let bytes = build_test_vpk(&["a/b.txt"]);
        assert!(list_files_from_bytes(&bytes[..bytes.len() - 5]).is_err());
    }
}
